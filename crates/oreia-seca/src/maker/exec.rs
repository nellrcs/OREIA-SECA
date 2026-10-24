//! Execução das fases de uma tarefa e, para itens do Forgejo, o ciclo
//! clonar → trabalhar → commit/push → abrir PR → comentar na issue.
//!
//! Cada fase roda pela escada de escalada do papel (`[escalation.<papel>]`):
//! um papel sem entrada aí usa um degrau só, com o executor padrão e o modelo
//! do próprio papel — ou seja, sem `[escalation]` o comportamento é idêntico
//! ao de antes desta escada existir.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oreia_core::approval::ApprovalOutcome;
use oreia_core::{PhaseStatus, Task, TaskStatus};
use oreia_llm::ModelSpec;

use super::Maker;
use crate::config::{EscalationConfig, EscalationStep};
use crate::executor::{self, FailureContext, ModelSelector, PhaseExecutor, PhaseOutcome, PhaseRequest};
use crate::gitops;

/// O que não muda fase a fase dentro de uma tarefa: onde ela roda e o
/// contexto do repositório. Agrupado para não estourar o número de
/// parâmetros de `phase_request`/`run_escalation`.
struct PhaseCtx<'a> {
    workdir: &'a Path,
    agent_prompt: Option<&'a str>,
    base_branch: &'a str,
}

impl Maker {
    pub(crate) async fn execute(self: Arc<Self>, task: Task) -> anyhow::Result<()> {
        let work = task.work_item.clone();
        let result = self.clone().execute_inner(task).await;
        if let Some(w) = &work {
            self.release_work(w);
        }
        result
    }

    async fn execute_inner(self: Arc<Self>, mut task: Task) -> anyhow::Result<()> {
        let workdir: PathBuf = self.cfg.paths.workspace.join(&task.id);
        let role = task.model_role.clone().unwrap_or_else(|| "executor".into());
        let agent_prompt = task.work_item.as_ref().and_then(|w| self.forgejo.as_ref()?.agent_prompt(&w.agent));

        // Prepara o diretório de trabalho
        let mut start_sha: Option<String> = None;
        if let Some(w) = task.work_item.clone() {
            let forge = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado"))?;
            if workdir.exists() {
                tokio::fs::remove_dir_all(&workdir).await?;
            }
            if let Err(e) = forge.prepare_repo(&w, &workdir).await {
                task.set_status(TaskStatus::Failed);
                self.store.save(&task).await?;
                self.reply(
                    task.source,
                    &task.user_id,
                    &format!("✗ {}: não consegui preparar o repositório: {e:#}", task.id),
                )
                .await;
                return Err(e);
            }
            start_sha = gitops::current_sha(&workdir).await.ok();
            forge
                .comment_as_agent(&w, &format!("🤖 Comecei a trabalhar nesta issue.\n\nPlano:\n{}", task.plan_text()))
                .await;
        } else {
            tokio::fs::create_dir_all(&workdir).await?;
            let info = format!(
                "# Tarefa {}\n\n> {}\n\nCriada em {} via {}\n\n## Fases\n{}\n",
                task.id,
                task.goal,
                task.created_at.format("%d/%m/%Y %H:%M UTC"),
                task.source,
                task.plan_text()
            );
            tokio::fs::write(workdir.join("OREIA_SECA_TAREFA.md"), info).await?;
        }

        task.set_status(TaskStatus::Running);
        self.store.save(&task).await?;

        let base_branch = self.forgejo.as_ref().map(|f| f.cfg.base_branch.clone()).unwrap_or_else(|| "main".into());
        let ctx = PhaseCtx { workdir: &workdir, agent_prompt: agent_prompt.as_deref(), base_branch: &base_branch };

        let total = task.phases.len();
        for idx in 0..total {
            if task.phases[idx].status != PhaseStatus::Pending {
                continue;
            }
            let name = task.phases[idx].name.clone();
            self.reply(task.source, &task.user_id, &format!("▸ [{}/{total}] {name}...", idx + 1)).await;

            match self.run_phase_ladder(&mut task, idx, &role, &ctx).await {
                Ok(outcome) => {
                    let errs = outcome.action_errors;
                    task.phases[idx].summary = Some(outcome.summary);
                    task.phases[idx].status = PhaseStatus::Done;
                    let mark = if errs > 0 { format!("⚠ ({errs} ação(ões) com erro)") } else { "✓".into() };
                    self.reply(task.source, &task.user_id, &format!("{mark} {name}")).await;
                }
                Err(e) => {
                    tracing::error!(task = %task.id, phase = %name, error = %e, "fase falhou");
                    task.phases[idx].status = PhaseStatus::Failed;
                    task.phases[idx].summary =
                        Some(format!("ERRO: {}", e.to_string().chars().take(100).collect::<String>()));
                    self.reply(task.source, &task.user_id, &format!("✗ {name}: {e:#}")).await;
                }
            }
            task.updated_at = chrono::Utc::now();
            self.store.save(&task).await?;
        }

        // Junta os commits de checkpoint da escalada num só, para o PR sair limpo.
        if let Some(sha) = &start_sha {
            if let Err(e) = gitops::squash_since(&workdir, sha).await {
                tracing::warn!(task = %task.id, error = %e, "falha ao achatar commits de checkpoint");
            }
        }

        let failed = task.failed_count();
        task.set_status(if failed > 0 { TaskStatus::DoneWithErrors } else { TaskStatus::Done });

        let mut final_msg = if failed > 0 {
            format!("✅ {} concluída com {failed} fase(s) com erro.", task.id)
        } else {
            format!("✅ {} concluída!", task.id)
        };

        if let Some(w) = task.work_item.clone() {
            let forge = self.forgejo.as_ref().expect("checado acima");
            match forge.publish(&task, &w, &workdir).await {
                Ok(Some(url)) => {
                    task.pull_request = Some(url.clone());
                    final_msg.push_str(&format!("\n🔀 PR pronto para sua revisão: {url}"));
                }
                Ok(None) => final_msg.push_str("\nNenhuma alteração no código, PR não aberto."),
                Err(e) => final_msg.push_str(&format!("\n⚠ Falha ao publicar no Forgejo: {e:#}")),
            }
        }

        self.store.save(&task).await?;
        self.reply(task.source, &task.user_id, &final_msg).await;
        Ok(())
    }

    /// Resolve os degraus do papel e tenta a fase. Sem `[escalation.<papel>]`,
    /// é um degrau só com o executor padrão e o modelo do papel — o
    /// comportamento de antes da escada existir, incluindo o retry de
    /// infraestrutura (`phase_retries`) e nenhuma verificação por `test_cmd`.
    async fn run_phase_ladder(
        &self,
        task: &mut Task,
        idx: usize,
        role: &str,
        ctx: &PhaseCtx<'_>,
    ) -> anyhow::Result<PhaseOutcome> {
        match self.cfg.escalation.get(role).cloned() {
            None => {
                let exec = self.executor_for(&self.cfg.executor.default)?;
                let req = self.phase_request(task, idx, ctx, ModelSelector::Role(role.to_string()), None, None);
                executor::run_with_retries(exec.as_ref(), &req, self.cfg.executor.phase_retries).await
            }
            Some(esc) => self.run_escalation(task, idx, role, &esc, ctx).await,
        }
    }

    fn phase_request(
        &self,
        task: &Task,
        idx: usize,
        ctx: &PhaseCtx<'_>,
        selector: ModelSelector,
        failure: Option<FailureContext>,
        test_cmd: Option<String>,
    ) -> PhaseRequest {
        let done: Vec<String> =
            task.done_phases().map(|p| format!("- {}: {}", p.name, p.summary.as_deref().unwrap_or("ok"))).collect();
        let phase = &task.phases[idx];
        PhaseRequest {
            task_id: task.id.clone(),
            workdir: ctx.workdir.to_path_buf(),
            goal: task.goal.clone(),
            completed_summary: if done.is_empty() { "(none)".to_string() } else { done.join("\n") },
            phase_name: phase.name.clone(),
            phase_instruction: phase.instruction.clone(),
            phase_index: idx + 1,
            phase_total: task.phases.len(),
            agent_prompt: ctx.agent_prompt.map(String::from),
            work_item: task.work_item.clone(),
            base_branch: ctx.base_branch.to_string(),
            selector,
            failure,
            shell_timeout: Duration::from_secs(self.cfg.executor.shell_timeout_secs),
            step_timeout: Duration::from_secs(self.cfg.executor.step_timeout_secs),
            test_cmd,
        }
    }

    fn executor_for(&self, name: &str) -> anyhow::Result<Arc<dyn PhaseExecutor>> {
        if name == "builtin" {
            let e: Arc<dyn PhaseExecutor> = Arc::new(executor::builtin::BuiltinExecutor::new(
                self.router.clone(),
                self.actions(),
                self.cfg.executor.max_steps,
            ));
            return Ok(e);
        }
        let template = self
            .cfg
            .executors
            .get(name)
            .map(|p| p.command.clone())
            .or_else(|| executor::external::builtin_preset(name).map(String::from))
            .ok_or_else(|| anyhow::anyhow!("executor \"{name}\" não configurado"))?;
        let e: Arc<dyn PhaseExecutor> = Arc::new(executor::external::ExternalExecutor::new(name, template));
        Ok(e)
    }

    fn selector_for_step(&self, step: &EscalationStep) -> anyhow::Result<ModelSelector> {
        let spec = self
            .cfg
            .models
            .get(&step.model)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("modelo \"{}\" não existe em [models]", step.model))?;
        Ok(ModelSelector::Pinned { key: step.model.clone(), spec })
    }

    /// Tenta cada degrau de `esc.steps` até um passar a verificação, pedindo
    /// aprovação a partir de `ask_before_step` e parando em `max_cost_usd`.
    /// Cada tentativa falha é desfeita antes do degrau seguinte, e cada
    /// degrau que passa fica marcado por um checkpoint local (commit sem
    /// push), para o PR final sair com um commit só por fase.
    async fn run_escalation(
        &self,
        task: &mut Task,
        idx: usize,
        role: &str,
        esc: &EscalationConfig,
        ctx: &PhaseCtx<'_>,
    ) -> anyhow::Result<PhaseOutcome> {
        let workdir = ctx.workdir;
        let is_git = workdir.join(".git").exists();
        let phase_name = task.phases[idx].name.clone();
        let phase_instruction = task.phases[idx].instruction.clone();
        let phase_label = format!("{} fase {}/{}", task.id, idx + 1, task.phases.len());
        let mut failure: Option<FailureContext> = None;
        let mut total_cost = 0.0f64;
        let mut log_entries = Vec::new();
        let mut last_error = String::new();

        for (i, step) in esc.steps.iter().enumerate() {
            let step_no = i + 1;

            if let Some(max) = esc.max_cost_usd {
                if total_cost > max {
                    self.reply(
                        task.source,
                        &task.user_id,
                        &format!(
                            "💰 {phase_label} atingiu o teto de custo (US$ {total_cost:.2} > US$ {max:.2}); parando a escalada."
                        ),
                    )
                    .await;
                    break;
                }
            }

            if is_git && i > 0 {
                if let Err(e) = gitops::discard_attempt(workdir).await {
                    tracing::warn!(task = %task.id, error = %e, "falha ao descartar tentativa anterior");
                }
            }

            if let Some(ask_at) = esc.ask_before_step {
                if step_no >= ask_at {
                    let est = self.cfg.models.get(&step.model).map(|s| estimate_step_cost(s, step)).unwrap_or(0.0);
                    let question =
                        format!("⬆️ Subir {phase_label} para {} ({}) (~US$ {est:.2})?", step.executor, step.model);
                    let approval_id = format!("{}-p{}-s{step_no}", task.id, idx + 1);
                    for ch in self.human_channels() {
                        if let Err(e) = ch.ask_approval(&approval_id, &question).await {
                            tracing::warn!(channel = ch.name(), error = %e, "falha ao pedir aprovação de escalada");
                        }
                    }
                    let timeout = Duration::from_secs(self.cfg.approval.timeout_secs);
                    let outcome = self.approvals.request(&approval_id, &question, timeout).await;
                    let note = match outcome {
                        ApprovalOutcome::Approved => "✅ Aprovado, subindo de degrau.".to_string(),
                        ApprovalOutcome::Rejected => "❌ Rejeitado.".to_string(),
                        ApprovalOutcome::Expired => "⌛ Expirou sem resposta.".to_string(),
                    };
                    for ch in self.human_channels() {
                        ch.approval_resolved(&approval_id, &note).await;
                    }
                    if outcome != ApprovalOutcome::Approved {
                        self.log_escalation(&log_entries).await;
                        anyhow::bail!("{phase_label} rejeitada antes do degrau {step_no} ({})", step.executor);
                    }
                }
            }

            let exec = self.executor_for(&step.executor)?;
            let selector = self.selector_for_step(step)?;
            let req = self.phase_request(task, idx, ctx, selector, failure.clone(), esc.test_cmd.clone());

            if i == 0 {
                self.reply(
                    task.source,
                    &task.user_id,
                    &format!("⬆️ {phase_label}: tentando {} com {}", step.executor, step.model),
                )
                .await;
            } else {
                self.reply(
                    task.source,
                    &task.user_id,
                    &format!(
                        "⬆️ {phase_label}: testes falharam com {} → tentando {} ({step_no}/{})",
                        esc.steps[i - 1].model,
                        step.model,
                        esc.steps.len()
                    ),
                )
                .await;
            }

            let started = std::time::Instant::now();
            let result = executor::run_with_retries(exec.as_ref(), &req, self.cfg.executor.phase_retries).await;
            let verdict = executor::verify::verify(
                workdir,
                &phase_instruction,
                &result,
                esc.test_cmd.as_deref(),
                req.shell_timeout,
            )
            .await;
            let duration = started.elapsed();

            let (prompt_tokens, completion_tokens) =
                result.as_ref().map(|o| (o.prompt_tokens, o.completion_tokens)).unwrap_or((0, 0));
            let cost = estimate_cost(step, prompt_tokens, completion_tokens);
            total_cost += cost;

            let passed = matches!(verdict, executor::verify::Verdict::Passed);
            let reason = match &verdict {
                executor::verify::Verdict::Failed(f) => Some(f.error.clone()),
                executor::verify::Verdict::Passed => None,
            };
            log_entries.push(executor::log::EscalationLogEntry {
                task_id: task.id.clone(),
                role: role.to_string(),
                phase: idx + 1,
                step: step_no,
                executor: step.executor.clone(),
                model: step.model.clone(),
                passed,
                reason,
                duration_ms: duration.as_millis() as u64,
                prompt_tokens,
                completion_tokens,
                cost_usd: cost,
                at: chrono::Utc::now(),
            });

            if passed {
                if is_git {
                    if let Err(e) = gitops::checkpoint(workdir, &format!("wip: {phase_name} (degrau {step_no})")).await
                    {
                        tracing::warn!(task = %task.id, error = %e, "falha ao criar checkpoint local");
                    }
                }
                self.log_escalation(&log_entries).await;
                task.phases[idx].resolved_step = Some(oreia_core::StepResolution {
                    step: step_no,
                    executor: step.executor.clone(),
                    model: step.model.clone(),
                });
                return result;
            }

            if let executor::verify::Verdict::Failed(f) = verdict {
                last_error = f.error.clone();
                failure = Some(f);
            }
        }

        self.log_escalation(&log_entries).await;
        anyhow::bail!("esgotados os degraus de escalada ({} tentativa(s)): {last_error}", esc.steps.len())
    }

    async fn log_escalation(&self, entries: &[executor::log::EscalationLogEntry]) {
        if entries.is_empty() {
            return;
        }
        let path = self.cfg.paths.data.join("escalation.jsonl");
        if let Err(e) = executor::log::append(&path, entries).await {
            tracing::warn!(error = %e, "falha ao gravar data/escalation.jsonl");
        }
    }
}

/// Estimativa pré-tentativa (antes de saber os tokens reais), para a mensagem de aprovação.
fn estimate_step_cost(spec: &ModelSpec, step: &EscalationStep) -> f64 {
    let in_tokens = spec.context as f64;
    let out_tokens = spec.reserve_output as f64;
    (in_tokens / 1_000_000.0) * step.cost_per_mtok_in + (out_tokens / 1_000_000.0) * step.cost_per_mtok_out
}

/// Custo real de uma tentativa, a partir dos tokens que o modelo reportou.
fn estimate_cost(step: &EscalationStep, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    (prompt_tokens as f64 / 1_000_000.0) * step.cost_per_mtok_in
        + (completion_tokens as f64 / 1_000_000.0) * step.cost_per_mtok_out
}
