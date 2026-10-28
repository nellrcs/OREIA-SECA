//! Ferramentas do servidor MCP (ver `docs/mcp-plano/`).
//!
//! A lógica de cada ferramenta mora aqui, como métodos de [`Maker`] — não em
//! `http.rs` (só parse/serialize e autenticação) nem no crate `oreia-mcp`
//! (que fala com estes métodos só pela API HTTP interna, nunca acessa nada
//! direto). Nesta etapa (2), só as ferramentas de LEITURA têm lógica real;
//! as de escrita chegam nas etapas 4-7 — `http.rs` devolve 501 pra elas por
//! enquanto.

use oreia_core::TaskStatus;
use oreia_forgejo::{parse_agent_branch, split_repo, ForgejoClient};
use serde::{Deserialize, Serialize};

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
