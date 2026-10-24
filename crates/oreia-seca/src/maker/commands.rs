//! Comandos de barra, iguais em todos os canais (terminal, Telegram, REST).

use std::sync::Arc;

use oreia_core::{Incoming, PhaseStatus, TaskStatus, WorkItem};
use oreia_forgejo::{parse_agent_branch, split_repo, ReviewEvent};

use super::Maker;

pub const HELP: &str = "Comandos:
/status — fila, aprovações e modelos ativos
/queue — detalhes da fila
/tasks — suas últimas tarefas
/feature <descrição> — cria uma tarefa sem passar pelo classificador
/approve <id> · /reject <id> — decide tarefas pendentes
/pending — aprovações pendentes
/retry <id> — retoma uma tarefa interrompida
/models — modelo ativo por papel
/stats [papel] — estatísticas de escalada (degraus, custo)
/agents — agentes do Forgejo
/prs — PRs abertos aguardando você
/merge [dono/repo] <n> — aprova e integra um PR
/changes [dono/repo] <n> <texto> — pede ajustes; o agente volta ao PR
/skills [reload] — ações disponíveis
/resumir — resume e compacta esta conversa
/limpar — apaga o histórico desta conversa";

impl Maker {
    pub(crate) async fn command(self: Arc<Self>, msg: &Incoming, line: &str) -> anyhow::Result<()> {
        let mut parts = line.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let rest = parts.next().unwrap_or("").trim();
        let args: Vec<&str> = rest.split_whitespace().collect();
        let say = |t: String| {
            let me = self.clone();
            let (src, uid) = (msg.source, msg.user_id.clone());
            async move { me.reply(src, &uid, &t).await }
        };

        match cmd.as_str() {
            "help" | "ajuda" | "start" => say(HELP.into()).await,

            "approve" | "reject" => {
                let Some(id) = args.first() else {
                    say(format!("uso: /{cmd} <id>")).await;
                    return Ok(());
                };
                let ok = self.approvals.respond(id, cmd == "approve");
                if !ok {
                    say(format!("Nenhuma aprovação pendente para {id}.")).await;
                }
            }

            "pending" => {
                let list = self.approvals.list_pending();
                if list.is_empty() {
                    say("Nenhuma aprovação pendente.".into()).await;
                } else {
                    let t: Vec<String> = list.iter().map(|(id, q)| format!("• {id}: {q}")).collect();
                    say(format!("Aprovações pendentes:\n{}", t.join("\n"))).await;
                }
            }

            "status" => {
                let q = self.queue.status();
                let mut parts = Vec::new();
                if q.running > 0 {
                    parts.push(format!("{} executando", q.running));
                }
                if q.queued > 0 {
                    parts.push(format!("{} na fila", q.queued));
                }
                let pend = self.approvals.list_pending().len();
                if pend > 0 {
                    parts.push(format!("{pend} aguardando aprovação"));
                }
                let head = if parts.is_empty() { "Nenhuma tarefa ativa".to_string() } else { parts.join(", ") };

                let running = self.store.list_by_status(TaskStatus::Running).await.unwrap_or_default();
                let mut lines = vec![head];
                for t in running.iter().take(5) {
                    let done = t.phases.iter().filter(|p| p.status != PhaseStatus::Pending).count();
                    lines.push(format!(
                        "• {} — \"{}\" ({done}/{} fases)",
                        t.id,
                        t.goal.chars().take(45).collect::<String>(),
                        t.phases.len()
                    ));
                }
                lines.push(String::new());
                lines.push(models_text(&self));
                say(lines.join("\n")).await;
            }

            "queue" | "fila" => {
                let s = self.queue.status();
                say(format!(
                    "Fila: {} rodando, {} aguardando\nConcorrência: {}\nTotal: {} concluídas, {} com erro",
                    s.running, s.queued, s.concurrency, s.completed, s.failed
                ))
                .await;
            }

            "tasks" | "tarefas" => {
                let tasks = self.store.list_by_user(&msg.user_id).await?;
                if tasks.is_empty() {
                    say("Nenhuma tarefa encontrada.".into()).await;
                } else {
                    let t: Vec<String> = tasks
                        .iter()
                        .take(8)
                        .map(|t| {
                            let pr = t.pull_request.as_deref().map(|u| format!(" → {u}")).unwrap_or_default();
                            format!(
                                "[{}] {} ({}){pr}",
                                t.status.as_str(),
                                t.goal.chars().take(50).collect::<String>(),
                                t.id
                            )
                        })
                        .collect();
                    say(format!("Últimas tarefas:\n{}", t.join("\n"))).await;
                }
            }

            "feature" | "tarefa" => {
                if rest.is_empty() {
                    say("uso: /feature <descrição do que fazer>".into()).await;
                } else {
                    return self.start_task(msg, rest).await;
                }
            }

            "retry" => {
                let Some(id) = args.first() else {
                    say("uso: /retry <id>".into()).await;
                    return Ok(());
                };
                let Some(mut task) = self.store.load(id).await? else {
                    say(format!("Tarefa não encontrada: {id}")).await;
                    return Ok(());
                };
                if let Some(w) = &task.work_item {
                    if !self.claim_work(w) {
                        say(format!("{} já tem um agente trabalhando.", Maker::work_key(w))).await;
                        return Ok(());
                    }
                }
                for p in task.phases.iter_mut().filter(|p| p.status == PhaseStatus::Failed) {
                    p.status = PhaseStatus::Pending;
                }
                task.set_status(TaskStatus::Running);
                self.store.save(&task).await?;
                say(format!("🔁 Retomando {}: {}", task.id, task.goal.chars().take(60).collect::<String>())).await;
                self.enqueue(task);
            }

            "models" | "modelos" => say(models_text(&self)).await,

            "stats" | "estatisticas" | "estatísticas" => {
                let path = self.cfg.paths.data.join("escalation.jsonl");
                let entries = crate::executor::log::load(&path).await.unwrap_or_default();
                let stats = crate::executor::log::aggregate(&entries, args.first().copied());
                if stats.is_empty() {
                    say("Sem dados de escalada ainda.".into()).await;
                } else {
                    let lines: Vec<String> = stats
                        .iter()
                        .map(|s| {
                            let steps: Vec<String> = s
                                .by_step
                                .iter()
                                .map(|(step, n)| {
                                    let pct = if s.resolved_phases > 0 {
                                        *n as f64 / s.resolved_phases as f64 * 100.0
                                    } else {
                                        0.0
                                    };
                                    format!("degrau {step}: {pct:.0}%")
                                })
                                .collect();
                            format!(
                                "• {} — {} fase(s) resolvida(s) ({}), custo total US$ {:.2}, médio US$ {:.2}/tarefa",
                                s.role,
                                s.resolved_phases,
                                steps.join(", "),
                                s.total_cost_usd,
                                s.avg_cost_usd_per_task
                            )
                        })
                        .collect();
                    say(format!("📊 Estatísticas de escalada:\n{}", lines.join("\n"))).await;
                }
            }

            "agents" | "agentes" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                let lines: Vec<String> = self
                    .cfg
                    .agents
                    .iter()
                    .map(|a| {
                        let on = if f.agents.contains_key(&a.name) { "🟢" } else { "🔴 sem token" };
                        let model = self.router.active_model(a.role()).unwrap_or_else(|| "nenhum".into());
                        format!("{on} {} — label `{}`, usuário {}, modelo {model}", a.name, a.label, a.user)
                    })
                    .collect();
                say(if lines.is_empty() { "Nenhum agente em [[agents]].".into() } else { lines.join("\n") }).await;
            }

            "prs" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                let Some(owner) = &f.owner else {
                    say(format!("Defina {} para usar /prs.", f.cfg.owner_token_env)).await;
                    return Ok(());
                };
                let mut lines = Vec::new();
                for full in &f.cfg.repos {
                    let Some((o, r)) = split_repo(full) else { continue };
                    match owner.list_open_pulls(o, r).await {
                        Ok(prs) => lines.extend(prs.into_iter().map(|p| {
                            let by = p.user.map(|u| u.login).unwrap_or_default();
                            format!("• {full}#{} {} (por {by})\n  {}", p.number, p.title, p.html_url)
                        })),
                        Err(e) => lines.push(format!("⚠ {full}: {e}")),
                    }
                }
                say(if lines.is_empty() {
                    "Nenhum PR aberto. 🎉".into()
                } else {
                    format!("PRs abertos:\n{}", lines.join("\n"))
                })
                .await;
            }

            "merge" => {
                let (o, r, n, _) = match self.repo_and_number(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        say(format!("{e}\nuso: /merge [dono/repo] <n>")).await;
                        return Ok(());
                    }
                };
                let f = self.forgejo.as_ref().expect("checado em repo_and_number");
                let owner = f.owner.as_ref().expect("checado em repo_and_number");
                if let Err(e) = owner.review_pull(&o, &r, n, ReviewEvent::Approve, "Aprovado via OREIA-SECA.").await {
                    tracing::warn!(error = %e, "não consegui registrar a aprovação (seguindo para o merge)");
                }
                owner.merge_pull(&o, &r, n, &f.cfg.merge_style).await?;
                say(format!("✅ {o}/{r}#{n} integrado.")).await;
            }

            "changes" | "ajustes" => {
                let (o, r, n, text) = match self.repo_and_number(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        say(format!("{e}\nuso: /changes [dono/repo] <n> <o que mudar>")).await;
                        return Ok(());
                    }
                };
                if text.is_empty() {
                    say("Diga o que precisa mudar: /changes [dono/repo] <n> <texto>".into()).await;
                    return Ok(());
                }
                let f = self.forgejo.as_ref().expect("checado em repo_and_number");
                let owner = f.owner.as_ref().expect("checado em repo_and_number");
                let pr = owner.get_pull(&o, &r, n).await?;
                owner.review_pull(&o, &r, n, ReviewEvent::RequestChanges, &text).await?;

                match pr.head.as_ref().and_then(|h| parse_agent_branch(&h.name)) {
                    Some((agent, issue)) if f.agents.contains_key(agent) => {
                        let item = WorkItem {
                            owner: o.clone(),
                            repo: r.clone(),
                            issue,
                            agent: agent.to_string(),
                            title: pr.title.clone(),
                        };
                        say(format!("📝 Pedido de ajuste registrado. {agent} vai voltar ao PR #{n}.")).await;
                        let goal = format!("{}\n\nAjustes pedidos na revisão do PR #{n}:\n{text}", pr.title);
                        self.clone().dispatch_work(item, goal, &msg.user_id).await;
                    }
                    _ => say(format!("📝 Pedido registrado no PR #{n} (não é um branch de agente ativo).")).await,
                }
            }

            "skills" => {
                if args.first() == Some(&"reload") {
                    let n = self.reload_skills();
                    say(format!("✅ Skills recarregadas: {n} skill(s) além das ações embutidas.")).await;
                } else {
                    let lines: Vec<String> = self
                        .actions()
                        .describe()
                        .into_iter()
                        .map(|(name, desc, params)| {
                            let ps: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                            let ps = if ps.is_empty() { String::new() } else { format!(" ({})", ps.join(", ")) };
                            format!("• {name}{ps}: {}", desc.lines().next().unwrap_or(""))
                        })
                        .collect();
                    say(format!("🛠 Ações e skills:\n{}\n\nUse /skills reload após editar skills/.", lines.join("\n")))
                        .await;
                }
            }

            "resumir" => {
                let file = self.summarize_session(&msg.session_key()).await?;
                say(format!("✅ Conversa resumida e salva em {file}.")).await;
            }

            "limpar" | "clear" => {
                self.sessions.clear(&msg.session_key());
                self.sessions.flush()?;
                say("🧹 Histórico desta conversa apagado.".into()).await;
            }

            _ => say(format!("Comando desconhecido: /{cmd}\n\n{HELP}")).await,
        }
        Ok(())
    }

    /// Lê `[dono/repo] <n> [texto…]`. O repositório pode ser omitido se só há um em `[forgejo].repos`.
    fn repo_and_number(&self, args: &[&str]) -> anyhow::Result<(String, String, u64, String)> {
        let f = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado."))?;
        anyhow::ensure!(f.owner.is_some(), "Defina {} para usar este comando.", f.cfg.owner_token_env);

        let (repo, rest) = match args.first().and_then(|a| split_repo(a)) {
            Some((o, r)) => ((o.to_string(), r.to_string()), &args[1..]),
            None => match f.cfg.repos.as_slice() {
                [only] => {
                    let (o, r) =
                        split_repo(only).ok_or_else(|| anyhow::anyhow!("repo inválido em [forgejo].repos: {only}"))?;
                    ((o.to_string(), r.to_string()), args)
                }
                _ => anyhow::bail!("Informe o repositório (dono/repo)."),
            },
        };
        let n: u64 = rest
            .first()
            .and_then(|s| s.trim_start_matches('#').parse().ok())
            .ok_or_else(|| anyhow::anyhow!("Informe o número do PR."))?;
        Ok((repo.0, repo.1, n, rest[1..].join(" ")))
    }
}

fn models_text(m: &Maker) -> String {
    let roles: Vec<String> = m
        .router
        .describe()
        .into_iter()
        .map(|r| format!("{} {} → {}", if r.ready { "🟢" } else { "🔴" }, r.role, r.model))
        .collect();
    let models: Vec<String> = m
        .router
        .describe_models()
        .into_iter()
        .map(|mi| {
            let icon = match mi.state {
                oreia_llm::ModelState::Ok => "🟢",
                oreia_llm::ModelState::Unreachable => "🔴",
                oreia_llm::ModelState::NotConfigured => "⚪",
            };
            if mi.state == oreia_llm::ModelState::NotConfigured {
                format!("{icon} {}: {}", mi.key, mi.state.as_str())
            } else {
                format!("{icon} {}: {} — {} ({})", mi.key, mi.state.as_str(), mi.model, mi.base_url)
            }
        })
        .collect();
    format!("Modelos por papel:\n{}\n\nModelos configurados:\n{}", roles.join("\n"), models.join("\n"))
}
