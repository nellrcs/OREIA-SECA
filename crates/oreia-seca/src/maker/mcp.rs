//! Ferramentas do servidor MCP (ver `docs/mcp-plano/`).
//!
//! A lógica de cada ferramenta mora aqui, como métodos de [`Maker`] — não em
//! `http.rs` (só parse/serialize e autenticação) nem no crate `oreia-mcp`
//! (que fala com estes métodos só pela API HTTP interna, nunca acessa nada
//! direto). `http.rs` devolve 501 pras ferramentas de escrita que ainda não
//! chegaram (etapas 5-7).

use oreia_core::TaskStatus;
use oreia_forgejo::{parse_agent_branch, split_repo, ForgejoClient};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::{ClaimStore, Maker, Owner};

#[derive(Debug, Deserialize)]
pub struct ListStoriesReq {
    pub repo: Option<String>,
    /// `all` (padrão) | `available` | `in_progress` | `done`.
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StorySummary {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub status: String,
    /// `agent:<nome>` | `mcp` | ausente (livre).
    pub owner: Option<String>,
    pub task_status: Option<String>,
    pub pull_request: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RepoIssueReq {
    pub repo: String,
    pub n: u64,
}

#[derive(Debug, Serialize)]
pub struct PhaseView {
    pub name: String,
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetStoryResp {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub owner: Option<String>,
    pub task_id: Option<String>,
    pub task_status: Option<String>,
    pub phases: Vec<PhaseView>,
    pub escalation: String,
    pub pull_request: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetPrResp {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub state: String,
    pub files_changed: Vec<String>,
    pub diff: String,
    /// `REVIEW.md` do diretório de trabalho do MCP, se existir (ver etapa 5).
    pub review_notes: Option<String>,
    pub escalation: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateStoryReq {
    pub repo: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub likely_files: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Option<String>,
    /// Aplica a label do agente e dispara a via automática. Ausente = issue livre.
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateStoryResp {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub agent: Option<String>,
}

/// Limite de caracteres do diff devolvido por `get_pr` — mesma ideia de
/// `DIFF_MAX_CHARS` em `executor::verify`, mas maior (é pra leitura humana/P.O.,
/// não pra caber num prompt de modelo).
const PR_DIFF_MAX_CHARS: usize = 8_000;

impl Maker {
    fn owner_client(&self) -> anyhow::Result<&ForgejoClient> {
        let forge = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado"))?;
        forge.owner.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "token do dono ausente ({}); defina-o pra usar as ferramentas MCP",
                forge.cfg.owner_token_env
            )
        })
    }

    fn target_repos(&self, repo: &Option<String>) -> anyhow::Result<Vec<(String, String)>> {
        if let Some(r) = repo {
            let (o, r2) = split_repo(r).ok_or_else(|| anyhow::anyhow!("repo inválido: {r} (use dono/repo)"))?;
            return Ok(vec![(o.to_string(), r2.to_string())]);
        }
        let forge = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado"))?;
        Ok(forge.cfg.repos.iter().filter_map(|f| split_repo(f)).map(|(o, r)| (o.to_string(), r.to_string())).collect())
    }

    fn owner_label(owner: &Owner) -> String {
        match owner {
            Owner::Agent { name } => format!("agent:{name}"),
            Owner::Mcp => "mcp".to_string(),
        }
    }

    /// Resolve o ID da label de um agente num repo — a API do Forgejo só aceita ID (não
    /// nome) pra criar/marcar issue com label. Mesma lógica de `create_issue_and_reply`/
    /// `assign_and_reply` em `maker/commands.rs` (o `/issue` do Telegram), reescrita aqui
    /// pra não acoplar o comando de Telegram ao MCP — unificar os três fica pra uma
    /// limpeza futura, não é pré-requisito desta etapa.
    async fn agent_label_id(
        &self,
        owner_client: &ForgejoClient,
        o: &str,
        r: &str,
        agent_name: &str,
    ) -> anyhow::Result<i64> {
        let agent = self.cfg.agents.iter().find(|a| a.name == agent_name).ok_or_else(|| {
            anyhow::anyhow!(
                "agente \"{agent_name}\" não existe. Agentes: {}",
                self.cfg.agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        })?;
        let labels = owner_client.list_labels(o, r).await?;
        labels.iter().find(|l| l.name == agent.label).map(|l| l.id).ok_or_else(|| {
            anyhow::anyhow!(
                "label \"{}\" não existe em {o}/{r}. Crie-a no Forgejo antes (Issues → Labels).",
                agent.label
            )
        })
    }

    /// Registro de auditoria (`data/mcp.jsonl`) de toda ferramenta que ESCREVE — nunca
    /// conteúdo sensível (nem token, nem corpo da issue/comentário), só o necessário pra
    /// rastrear o que uma ferramenta MCP fez. Ver seção de Segurança do plano geral.
    async fn log_mcp(&self, tool: &str, repo: &str, issue: Option<u64>, result: &str) {
        #[derive(Serialize)]
        struct Entry<'a> {
            tool: &'a str,
            repo: &'a str,
            issue: Option<u64>,
            result: &'a str,
            at: chrono::DateTime<chrono::Utc>,
        }
        let entry = Entry { tool, repo, issue, result, at: chrono::Utc::now() };
        let Ok(mut line) = serde_json::to_string(&entry) else { return };
        line.push('\n');
        let path = self.cfg.paths.data.join("mcp.jsonl");
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        match tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()).await {
                    tracing::warn!(error = %e, "falha ao gravar data/mcp.jsonl");
                }
            }
            Err(e) => tracing::warn!(error = %e, "falha ao abrir data/mcp.jsonl"),
        }
    }

    pub async fn list_stories(&self, req: ListStoriesReq) -> anyhow::Result<Vec<StorySummary>> {
        let owner_client = self.owner_client()?;
        let repos = self.target_repos(&req.repo)?;
        let tasks = self.store.list().await.unwrap_or_default();
        let status_filter = req.status.as_deref().unwrap_or("all");

        let mut out = Vec::new();
        for (o, r) in repos {
            let repo_full = format!("{o}/{r}");
            let issues = owner_client.list_issues(&o, &r, "all").await?;
            let open_prs = owner_client.list_open_pulls(&o, &r).await.unwrap_or_default();

            for issue in issues {
                let claim = self.claims.get(&repo_full, issue.number);
                let key = ClaimStore::key(&repo_full, issue.number);
                let last_task = tasks
                    .iter()
                    .filter(|t| t.work_item.as_ref().map(Maker::work_key).as_deref() == Some(key.as_str()))
                    .max_by_key(|t| t.updated_at);
                let pr = open_prs.iter().find(|p| {
                    p.head.as_ref().and_then(|h| parse_agent_branch(&h.name)).map(|(_, n)| n) == Some(issue.number)
                });
                let has_agent_label = self.cfg.agent_by_label(issue.labels.iter().map(|l| l.name.as_str())).is_some();

                // "available": livre (sem label de agente), agente falhou, ou tem PR
                // aberto esperando ajuste — qualquer um desses já é algo acionável pra
                // um P.O. olhar, então todos caem no mesmo balde.
                let failed_before =
                    matches!(last_task.map(|t| t.status), Some(TaskStatus::Failed | TaskStatus::DoneWithErrors));
                let computed = if claim.is_some() {
                    "in_progress"
                } else if failed_before || pr.is_some() || !has_agent_label {
                    "available"
                } else if matches!(last_task.map(|t| t.status), Some(TaskStatus::Done)) || issue.state == "closed" {
                    "done"
                } else {
                    "available"
                };

                if status_filter != "all" && status_filter != computed {
                    continue;
                }

                out.push(StorySummary {
                    repo: repo_full.clone(),
                    number: issue.number,
                    title: issue.title,
                    status: computed.to_string(),
                    owner: claim.map(|c| Self::owner_label(&c.owner)),
                    task_status: last_task.map(|t| t.status.as_str().to_string()),
                    pull_request: pr.map(|p| p.html_url.clone()),
                });
            }
        }
        Ok(out)
    }

    pub async fn get_story(&self, req: RepoIssueReq) -> anyhow::Result<GetStoryResp> {
        let (o, r) =
            split_repo(&req.repo).ok_or_else(|| anyhow::anyhow!("repo inválido: {} (use dono/repo)", req.repo))?;
        let owner_client = self.owner_client()?;
        let issue = owner_client.get_issue(o, r, req.n).await?;

        let claim = self.claims.get(&req.repo, req.n);
        let key = ClaimStore::key(&req.repo, req.n);
        let tasks = self.store.list().await.unwrap_or_default();
        let task = tasks
            .iter()
            .filter(|t| t.work_item.as_ref().map(Maker::work_key).as_deref() == Some(key.as_str()))
            .max_by_key(|t| t.updated_at);
        let open_prs = owner_client.list_open_pulls(o, r).await.unwrap_or_default();
        let pr = open_prs
            .iter()
            .find(|p| p.head.as_ref().and_then(|h| parse_agent_branch(&h.name)).map(|(_, n)| n) == Some(req.n));

        Ok(GetStoryResp {
            repo: req.repo.clone(),
            number: req.n,
            title: issue.title,
            body: issue.body,
            owner: claim.map(|c| Self::owner_label(&c.owner)),
            task_id: task.map(|t| t.id.clone()),
            task_status: task.map(|t| t.status.as_str().to_string()),
            phases: task
                .map(|t| {
                    t.phases
                        .iter()
                        .map(|p| PhaseView {
                            name: p.name.clone(),
                            status: p.status.as_str().to_string(),
                            summary: p.summary.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            escalation: task.map(|t| t.escalation_summary()).unwrap_or_default(),
            pull_request: pr.map(|p| p.html_url.clone()),
        })
    }

    pub async fn get_pr(&self, req: RepoIssueReq) -> anyhow::Result<GetPrResp> {
        let (o, r) =
            split_repo(&req.repo).ok_or_else(|| anyhow::anyhow!("repo inválido: {} (use dono/repo)", req.repo))?;
        let owner_client = self.owner_client()?;
        let pr = owner_client.get_pull(o, r, req.n).await?;
        let diff = owner_client.get_pull_diff(o, r, req.n).await.unwrap_or_default();

        let files_changed: Vec<String> = diff
            .lines()
            .filter_map(|l| l.strip_prefix("diff --git a/"))
            .filter_map(|rest| rest.split(" b/").next())
            .map(|s| s.to_string())
            .collect();
        let truncated: String = diff.chars().take(PR_DIFF_MAX_CHARS).collect();

        let review_notes = match &self.cfg.mcp {
            Some(mcp) => {
                let path = mcp.workdir.join(format!("{o}-{r}-{}", req.n)).join("REVIEW.md");
                tokio::fs::read_to_string(&path).await.ok()
            }
            None => None,
        };

        let mut escalation = String::new();
        if let Some(issue_n) = pr.head.as_ref().and_then(|h| parse_agent_branch(&h.name)).map(|(_, n)| n) {
            let key = ClaimStore::key(&req.repo, issue_n);
            let tasks = self.store.list().await.unwrap_or_default();
            if let Some(t) = tasks
                .iter()
                .filter(|t| t.work_item.as_ref().map(Maker::work_key).as_deref() == Some(key.as_str()))
                .max_by_key(|t| t.updated_at)
            {
                escalation = t.escalation_summary();
            }
        }

        Ok(GetPrResp {
            repo: req.repo,
            number: req.n,
            title: pr.title,
            author: pr.user.map(|u| u.login),
            state: pr.state,
            files_changed,
            diff: truncated,
            review_notes,
            escalation,
        })
    }

    pub async fn create_story(&self, req: CreateStoryReq) -> anyhow::Result<CreateStoryResp> {
        anyhow::ensure!(!req.acceptance_criteria.is_empty(), "defina pelo menos 1 critério de aceite");
        let (o, r) =
            split_repo(&req.repo).ok_or_else(|| anyhow::anyhow!("repo inválido: {} (use dono/repo)", req.repo))?;
        let owner_client = self.owner_client()?;

        let label_id = match &req.agent {
            Some(name) => Some(self.agent_label_id(owner_client, o, r, name).await?),
            None => None,
        };

        let mut body = format!("## Objetivo\n{}\n\n## Critérios de aceite\n", req.goal);
        for c in &req.acceptance_criteria {
            body.push_str(&format!("- [ ] {c}\n"));
        }
        if !req.likely_files.is_empty() {
            body.push_str("\n## Arquivos prováveis\n");
            for f in &req.likely_files {
                body.push_str(&format!("- {f}\n"));
            }
        }
        body.push_str(&format!("\n## Fora de escopo\n{}\n", req.out_of_scope.as_deref().unwrap_or("(nenhum)")));

        let label_ids: Vec<i64> = label_id.into_iter().collect();
        let issue = match owner_client.create_issue(o, r, &req.title, &body, &label_ids, None).await {
            Ok(issue) => issue,
            Err(e) => {
                self.log_mcp("create_story", &req.repo, None, "erro").await;
                return Err(e.into());
            }
        };
        self.log_mcp("create_story", &req.repo, Some(issue.number), "ok").await;

        Ok(CreateStoryResp {
            repo: req.repo,
            number: issue.number,
            title: issue.title,
            html_url: issue.html_url,
            agent: req.agent,
        })
    }

    pub async fn mcp_stats(&self, role: Option<String>) -> Vec<crate::executor::log::RoleStats> {
        let path = self.cfg.paths.data.join("escalation.jsonl");
        let entries = crate::executor::log::load(&path).await.unwrap_or_default();
        crate::executor::log::aggregate(&entries, role.as_deref())
    }

    /// Recursos MCP (`oreia://...`), somente leitura. `oreia://repos/<repo>/contract`
    /// ainda não tem fonte configurada — fica para quando `[mcp]` ganhar esse campo.
    pub async fn read_resource(&self, uri: &str) -> anyhow::Result<String> {
        if let Some(name) = uri.strip_prefix("oreia://agents/") {
            let forge = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado"))?;
            return forge
                .agent_prompt(name)
                .ok_or_else(|| anyhow::anyhow!("agente \"{name}\" não existe ou não tem AGENT.md configurado"));
        }
        if uri == "oreia://docs/roadmap" {
            return tokio::fs::read_to_string("docs/roadmap.md")
                .await
                .map_err(|e| anyhow::anyhow!("docs/roadmap.md: {e}"));
        }
        if let Some(repo) = uri.strip_prefix("oreia://repos/").and_then(|s| s.strip_suffix("/contract")) {
            anyhow::bail!("contrato de \"{repo}\" ainda não configurado");
        }
        anyhow::bail!("recurso desconhecido: {uri}")
    }
}

#[cfg(test)]
mod tests;
