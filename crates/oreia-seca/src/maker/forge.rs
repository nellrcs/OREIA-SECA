//! Lado Forgejo do orquestrador: agentes, eventos de webhook e publicação de PRs.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use oreia_core::{Incoming, Source, Task, TaskStatus, WorkItem, WorkMode};
use oreia_forgejo::webhook::Event;
use oreia_forgejo::{parse_agent_branch, ForgejoClient, ForgejoError};

use super::Maker;
use crate::config::{env, AgentConfig, Config, ForgejoConfig};
use crate::gitops::{self, GitAuth};

pub struct Agent {
    pub cfg: AgentConfig,
    pub client: ForgejoClient,
    token: String,
    pub prompt: Option<String>,
}

impl Agent {
    fn auth(&self) -> GitAuth {
        GitAuth::new(&self.cfg.user, &self.token)
    }
}

pub struct ForgejoCtx {
    pub cfg: ForgejoConfig,
    /// Cliente com o token do dono (você): /prs, /merge, /changes.
    pub owner: Option<ForgejoClient>,
    pub agents: BTreeMap<String, Agent>,
    pub webhook_secret: Option<String>,
}

impl ForgejoCtx {
    pub fn build(cfg: &Config) -> anyhow::Result<Option<Self>> {
        let Some(fc) = cfg.forgejo.clone() else { return Ok(None) };

        let owner = env(&fc.owner_token_env).map(|t| ForgejoClient::new(&fc.url, &t));
        if owner.is_none() {
            tracing::warn!(var = %fc.owner_token_env, "sem token do dono: /prs, /merge e /changes ficam desativados");
        }

        let mut agents = BTreeMap::new();
        for a in &cfg.agents {
            let Some(token) = env(&a.token_env) else {
                tracing::warn!(agent = %a.name, var = %a.token_env, "agente sem token, ignorado");
                continue;
            };
            let prompt = match &a.prompt {
                Some(p) => match std::fs::read_to_string(p) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!(agent = %a.name, file = %p.display(), error = %e, "AGENT.md não encontrado");
                        None
                    }
                },
                None => None,
            };
            agents.insert(
                a.name.clone(),
                Agent { client: ForgejoClient::new(&fc.url, &token), cfg: a.clone(), token, prompt },
            );
        }

        let webhook_secret = env(&fc.webhook_secret_env);
        if webhook_secret.is_none() {
            tracing::warn!(var = %fc.webhook_secret_env, "sem segredo de webhook: /webhooks/forgejo vai recusar eventos");
        }
        Ok(Some(Self { cfg: fc, owner, agents, webhook_secret }))
    }

    pub fn agent_names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    pub fn agent_prompt(&self, name: &str) -> Option<String> {
        self.agents.get(name)?.prompt.clone()
    }

    pub fn web_url(&self) -> &str {
        self.cfg.public_url.as_deref().unwrap_or(&self.cfg.url).trim_end_matches('/')
    }

    fn clone_url(&self, w: &WorkItem) -> String {
        let base = self.cfg.git_url.as_deref().unwrap_or(&self.cfg.url);
        format!("{}/{}/{}.git", base.trim_end_matches('/'), w.owner, w.repo)
    }

    fn agent(&self, name: &str) -> anyhow::Result<&Agent> {
        self.agents.get(name).ok_or_else(|| anyhow::anyhow!("agente \"{name}\" não está ativo (falta token?)"))
    }

    pub async fn prepare_repo(&self, w: &WorkItem, dir: &Path) -> anyhow::Result<()> {
        let agent = self.agent(&w.agent)?;
        gitops::prepare(
            dir,
            &self.clone_url(w),
            w.effective_base_branch(&self.cfg.base_branch),
            &w.checkout_branch(),
            &agent.auth(),
            &agent.cfg.user,
        )
        .await
    }

    /// Comenta na issue como o agente (erros só vão para o log).
    pub async fn comment_as_agent(&self, w: &WorkItem, body: &str) {
        let Ok(agent) = self.agent(&w.agent) else { return };
        if let Err(e) = agent.client.comment_issue(&w.owner, &w.repo, w.issue, body).await {
            tracing::warn!(error = %e, "falha ao comentar na issue");
        }
    }

    /// Commit, push e PR. Retorna a URL do PR, ou `None` se não houve mudança.
    pub async fn publish(&self, task: &Task, w: &WorkItem, dir: &Path) -> anyhow::Result<Option<String>> {
        let agent = self.agent(&w.agent)?;
        let title =
            if w.title.is_empty() { task.goal.lines().next().unwrap_or("").to_string() } else { w.title.clone() };
        let message = format!("{title} (#{})\n\nTarefa {} · agente {}", w.issue, task.id, w.agent);

        let pushed = gitops::commit_and_push(dir, &w.branch_name(), &message, &agent.auth()).await?;
        if !pushed {
            self.comment_as_agent(w, "Terminei, mas nenhuma alteração de código foi necessária.").await;
            return Ok(None);
        }

        let base = w.effective_base_branch(&self.cfg.base_branch);
        let stat = gitops::changed_files(dir, base).await;
        let phases: String = task
            .phases
            .iter()
            .map(|p| {
                format!(
                    "- [{}] **{}**: {}",
                    if p.status == oreia_core::PhaseStatus::Done { "x" } else { " " },
                    p.name,
                    p.summary.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let escalation: String = task
            .phases
            .iter()
            .filter_map(|p| {
                p.resolved_step
                    .as_ref()
                    .map(|r| format!("- **{}**: degrau {} ({} · {})", p.name, r.step, r.executor, r.model))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let escalation_section =
            if escalation.is_empty() { String::new() } else { format!("\n\n## Escalada\n{escalation}") };
        let body = format!(
            "Closes #{}\n\nAgente: `{}` · Tarefa: `{}`\n\n## Fases\n{phases}{escalation_section}\n\n## Arquivos alterados\n```\n{}\n```\n\n> PR aberto automaticamente. Revise, teste no preview e aprove.",
            w.issue,
            w.agent,
            task.id,
            stat.trim()
        );

        let branch = w.branch_name();
        let url = match agent
            .client
            .create_pull(&w.owner, &w.repo, &format!("{title} (#{})", w.issue), &body, &branch, base)
            .await
        {
            Ok(pr) => pr.html_url,
            // PR já existe para este branch: o push acima já o atualizou.
            Err(ForgejoError::Api { status: 409 | 422, .. }) => {
                let open = agent.client.list_open_pulls(&w.owner, &w.repo).await?;
                let pr = open
                    .into_iter()
                    .find(|p| p.head.as_ref().map(|h| h.name == branch).unwrap_or(false))
                    .ok_or_else(|| anyhow::anyhow!("o PR já existia mas não foi encontrado"))?;
                let _ = agent
                    .client
                    .comment_issue(
                        &w.owner,
                        &w.repo,
                        pr.number,
                        &format!("🔁 Atualizado pela tarefa `{}`.\n\n## Fases\n{phases}", task.id),
                    )
                    .await;
                pr.html_url
            }
            Err(e) => return Err(e.into()),
        };

        self.comment_as_agent(w, &format!("🔀 Abri/atualizei o PR: {url}")).await;
        Ok(Some(url))
    }

    /// Fecha uma revisão automática: sobe qualquer correção pequena que o
    /// revisor tenha feito (mesma branch do PR, nunca uma nova) e comenta o
    /// resultado no PR. Nunca chama `review_pull`/`merge_pull` — aprovar,
    /// reprovar e fazer merge são sempre comando do humano (`/merge`,
    /// `/changes`), nunca uma ação automática de agente.
    pub async fn publish_review(&self, task: &Task, w: &WorkItem, dir: &Path) -> anyhow::Result<()> {
        let agent = self.agent(&w.agent)?;
        let branch = w.checkout_branch();
        let message = format!("review: ajustes do {} no PR #{}\n\nTarefa {}", w.agent, w.issue, task.id);
        let pushed = gitops::commit_and_push(dir, &branch, &message, &agent.auth()).await?;

        let findings = task.phases.last().and_then(|p| p.summary.as_deref()).unwrap_or("(sem resumo)");
        let phase_names: String = task.phases.iter().map(|p| format!("- {}", p.name)).collect::<Vec<_>>().join("\n");
        let fixes_note = if pushed { "\n\n_Subi pequenos ajustes nesta mesma branch._" } else { "" };
        let body = format!(
            "## 🔎 Revisão automática ({})\n\n{findings}{fixes_note}\n\n<details><summary>Fases executadas</summary>\n\n{phase_names}\n\n</details>\n\n> Isto é um comentário, não uma aprovação ou reprovação — a decisão de merge é sempre sua (`/merge` ou `/changes`).",
            w.agent
        );

        agent.client.comment_issue(&w.owner, &w.repo, w.issue, &body).await?;
        Ok(())
    }
}

impl Maker {
    /// Já existe tarefa concluída (com ou sem erros) para este item?
    async fn already_delivered(&self, w: &WorkItem) -> bool {
        let key = Maker::work_key(w);
        self.store.list().await.unwrap_or_default().iter().any(|t| {
            t.work_item.as_ref().map(Maker::work_key).as_deref() == Some(key.as_str())
                && matches!(t.status, TaskStatus::Done | TaskStatus::DoneWithErrors)
        })
    }

    /// Coloca um agente para trabalhar num item do Forgejo.
    pub(crate) async fn dispatch_work(self: Arc<Self>, item: WorkItem, goal: String, requested_by: &str) {
        if !self.claim_work(&item) {
            self.broadcast(&format!(
                "ℹ️ {} já está trabalhando em {}#{}; pedido ignorado.",
                item.agent,
                item.repo_full_name(),
                item.issue
            ))
            .await;
            return;
        }
        let msg = Incoming::new(Source::Forgejo, requested_by, &goal).with_work_item(item);
        if let Err(e) = self.clone().start_task(&msg, &goal).await {
            tracing::error!(error = %e, "falha ao iniciar tarefa do Forgejo");
            self.broadcast(&format!("❌ Erro: {e:#}")).await;
        }
    }

    pub async fn handle_forgejo(self: Arc<Self>, event: Event) {
        let Some(forge) = &self.forgejo else { return };
        match event {
            Event::Issues(p) => {
                if !matches!(p.action.as_str(), "opened" | "reopened" | "label_updated" | "labeled") {
                    return;
                }
                if self.cfg.is_agent_user(&p.sender.login) {
                    tracing::debug!(sender = %p.sender.login, "evento gerado por agente, ignorado");
                    return;
                }
                let labels = p.issue.labels.iter().map(|l| l.name.as_str());
                let Some(agent) = self.cfg.agent_by_label(labels) else { return };
                if !forge.agents.contains_key(&agent.name) {
                    tracing::warn!(agent = %agent.name, "issue para agente sem token");
                    return;
                }
                let probe = WorkItem {
                    owner: p.repository.owner.login.clone(),
                    repo: p.repository.name.clone(),
                    issue: p.issue.number,
                    agent: agent.name.clone(),
                    title: String::new(),
                    mode: WorkMode::Code,
                    review_branch: None,
                    base_branch: None,
                };
                // Abrir a issue já com a label gera dois eventos; ignora o repetido.
                if self.is_active(&probe) {
                    tracing::debug!(issue = p.issue.number, "issue já em andamento, evento repetido ignorado");
                    return;
                }
                // Mudança de label numa issue já atendida (ex.: adicionar "bug") não reabre o trabalho.
                // Para pedir ajustes, use @menção ou /changes.
                if p.action != "opened" && self.already_delivered(&probe).await {
                    tracing::debug!(issue = p.issue.number, "issue já entregue, mudança de label ignorada");
                    return;
                }
                // Campo "Ramo" da issue no Forgejo (ex. `refs/heads/funcionalides`): quando
                // preenchido, o agente clona e abre o PR contra essa branch em vez da branch
                // padrão global — pra trabalho em cima de uma feature branch já existente.
                let base_branch = oreia_forgejo::branch_from_ref(&p.issue.git_ref).map(String::from);
                let item = WorkItem {
                    owner: p.repository.owner.login.clone(),
                    repo: p.repository.name.clone(),
                    issue: p.issue.number,
                    agent: agent.name.clone(),
                    title: p.issue.title.clone(),
                    mode: WorkMode::Code,
                    review_branch: None,
                    base_branch,
                };
                let goal = format!("{}\n\n{}", p.issue.title, p.issue.body).trim().to_string();
                self.broadcast(&format!(
                    "📥 Issue {}#{} \"{}\" atribuída a {}.\n{}/{}/issues/{}",
                    item.repo_full_name(),
                    item.issue,
                    item.title,
                    item.agent,
                    forge.web_url(),
                    item.repo_full_name(),
                    item.issue
                ))
                .await;
                self.dispatch_work(item, goal, &p.sender.login).await;
            }

            // "@agent-back ajuste X" numa issue ou num PR do agente
            Event::IssueComment(p) => {
                if p.action != "created" || self.cfg.is_agent_user(&p.comment.user.login) {
                    return;
                }
                let Some(agent) = forge.agents.values().find(|a| p.comment.body.contains(&format!("@{}", a.cfg.user)))
                else {
                    return;
                };
                let (owner, repo) = (p.repository.owner.login.clone(), p.repository.name.clone());

                let (issue, base_branch) = if p.is_pull {
                    let pr = match agent.client.get_pull(&owner, &repo, p.issue.number).await {
                        Ok(pr) => pr,
                        Err(e) => {
                            tracing::warn!(error = %e, "não consegui ler o PR comentado");
                            return;
                        }
                    };
                    let n = match pr.head.as_ref().and_then(|h| parse_agent_branch(&h.name)) {
                        Some((name, n)) if name == agent.cfg.name => n,
                        _ => return,
                    };
                    (n, pr.base.as_ref().map(|b| b.name.clone()))
                } else {
                    (p.issue.number, oreia_forgejo::branch_from_ref(&p.issue.git_ref).map(String::from))
                };

                let item = WorkItem {
                    owner,
                    repo,
                    issue,
                    agent: agent.cfg.name.clone(),
                    title: p.issue.title.clone(),
                    mode: WorkMode::Code,
                    review_branch: None,
                    base_branch,
                };
                let goal =
                    format!("{}\n\nAjuste pedido por @{}:\n{}", p.issue.title, p.comment.user.login, p.comment.body);
                self.dispatch_work(item, goal, &p.comment.user.login).await;
            }

            Event::PullRequest(p) => {
                if p.action == "closed" && p.pull_request.merged {
                    self.broadcast(&format!(
                        "🎉 PR {}#{} \"{}\" foi integrado.",
                        p.repository.full_name, p.number, p.pull_request.title
                    ))
                    .await;
                    return;
                }
                if p.action != "opened" {
                    return;
                }
                // Revisão automática: só para PR aberto por um dos nossos agentes
                // (branch "agent/<nome>/issue-<n>"), e só se "reviewer" tiver token.
                if !self.cfg.is_agent_user(&p.sender.login) {
                    return;
                }
                let Some(head) = p.pull_request.head.as_ref() else { return };
                if parse_agent_branch(&head.name).is_none() {
                    return;
                }
                let Some(reviewer) = forge.agents.get("reviewer") else {
                    tracing::debug!(pr = p.number, "PR de agente aberto, mas \"reviewer\" não está configurado");
                    return;
                };
                // Base real do PR (pode não ser a branch padrão global, quando a issue
                // original apontava pra uma feature branch) — o reviewer precisa comparar
                // o diff contra ela, não contra um palpite.
                let item = WorkItem {
                    owner: p.repository.owner.login.clone(),
                    repo: p.repository.name.clone(),
                    issue: p.number,
                    agent: reviewer.cfg.name.clone(),
                    title: p.pull_request.title.clone(),
                    mode: WorkMode::Review,
                    review_branch: Some(head.name.clone()),
                    base_branch: p.pull_request.base.as_ref().map(|b| b.name.clone()),
                };
                if self.is_active(&item) || self.already_delivered(&item).await {
                    tracing::debug!(pr = p.number, "PR já revisado (ou em revisão), evento repetido ignorado");
                    return;
                }
                self.broadcast(&format!(
                    "🔎 PR {}#{} \"{}\" aberto; chamando reviewer.",
                    p.repository.full_name, p.number, p.pull_request.title
                ))
                .await;
                let goal =
                    format!("Review pull request #{} (\"{}\") before the human does.", p.number, p.pull_request.title);
                self.dispatch_work(item, goal, &p.sender.login).await;
            }

            Event::Other(name) => tracing::debug!(event = %name, "evento do Forgejo ignorado"),
        }
    }
}
