//! Integração com o Forgejo.
//!
//! - [`ForgejoClient`]: chamadas à API v1 (comentar issue, abrir PR, listar,
//!   revisar e fazer merge). Cada agente usa o próprio token, então o
//!   histórico mostra qual "pessoa" fez cada coisa.
//! - [`webhook`]: validação de assinatura HMAC e parsing dos eventos.

pub mod webhook;

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ForgejoError {
    #[error("requisição falhou: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Forgejo respondeu {status}: {body}")]
    Api { status: u16, body: String },
}

pub type Result<T> = std::result::Result<T, ForgejoError>;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub html_url: String,
    /// Branch escolhida no campo "Ramo" do Forgejo ao abrir a issue (ex.
    /// `refs/heads/funcionalides`), vazio quando a issue não aponta pra nenhuma. Veja
    /// [`branch_from_ref`].
    #[serde(rename = "ref", default)]
    pub git_ref: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BranchRef {
    #[serde(rename = "ref")]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub head: Option<BranchRef>,
    /// Branch base do PR (ex. `main`, ou outra quando a issue original apontava pra uma
    /// branch de feature via [`Issue::git_ref`]) — usada pelo `reviewer` pra comparar o
    /// diff certo, em vez de assumir sempre a branch padrão global.
    #[serde(default)]
    pub base: Option<BranchRef>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub merged: bool,
}

/// Branches de agente seguem `agent/<nome>/issue-<n>`. Devolve `(nome, n)`.
pub fn parse_agent_branch(branch: &str) -> Option<(&str, u64)> {
    let rest = branch.strip_prefix("agent/")?;
    let (agent, issue) = rest.split_once("/issue-")?;
    (!agent.is_empty() && !agent.contains('/')).then_some(())?;
    Some((agent, issue.parse().ok()?))
}

/// Extrai o nome da branch de um `ref` de issue do Forgejo (`refs/heads/x` → `x`), ou
/// `None` quando a issue não aponta pra nenhuma branch (o campo vem vazio).
pub fn branch_from_ref(git_ref: &str) -> Option<&str> {
    let r = git_ref.trim();
    if r.is_empty() {
        return None;
    }
    Some(r.strip_prefix("refs/heads/").unwrap_or(r))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

impl ReviewEvent {
    fn as_api(&self) -> &'static str {
        match self {
            ReviewEvent::Approve => "APPROVED",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
            ReviewEvent::Comment => "COMMENT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForgejoClient {
    http: reqwest::Client,
    api: String,
    token: String,
}

impl ForgejoClient {
    /// `base_url` sem `/api/v1` (ex.: `http://forgejo:3000`).
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            api: format!("{}/api/v1", base_url.trim_end_matches('/')),
            token: token.to_string(),
        }
    }

    pub fn api_url(&self) -> &str {
        &self.api
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.api, path))
            .header("Authorization", format!("token {}", self.token))
            .header("Accept", "application/json")
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(ForgejoError::Api { status: status.as_u16(), body: body.chars().take(500).collect() })
        }
    }

    pub async fn whoami(&self) -> Result<User> {
        let r = Self::check(self.req(reqwest::Method::GET, "/user").send().await?).await?;
        Ok(r.json().await?)
    }

    pub async fn get_issue(&self, owner: &str, repo: &str, index: u64) -> Result<Issue> {
        let r = self.req(reqwest::Method::GET, &format!("/repos/{owner}/{repo}/issues/{index}")).send().await?;
        Ok(Self::check(r).await?.json().await?)
    }

    pub async fn comment_issue(&self, owner: &str, repo: &str, index: u64, body: &str) -> Result<()> {
        let r = self
            .req(reqwest::Method::POST, &format!("/repos/{owner}/{repo}/issues/{index}/comments"))
            .json(&json!({ "body": body }))
            .send()
            .await?;
        Self::check(r).await?;
        Ok(())
    }

    pub async fn create_pull(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest> {
        let r = self
            .req(reqwest::Method::POST, &format!("/repos/{owner}/{repo}/pulls"))
            .json(&json!({ "title": title, "body": body, "head": head, "base": base }))
            .send()
            .await?;
        Ok(Self::check(r).await?.json().await?)
    }

    pub async fn get_pull(&self, owner: &str, repo: &str, index: u64) -> Result<PullRequest> {
        let r = self.req(reqwest::Method::GET, &format!("/repos/{owner}/{repo}/pulls/{index}")).send().await?;
        Ok(Self::check(r).await?.json().await?)
    }

    pub async fn list_open_pulls(&self, owner: &str, repo: &str) -> Result<Vec<PullRequest>> {
        let r = self
            .req(reqwest::Method::GET, &format!("/repos/{owner}/{repo}/pulls"))
            .query(&[("state", "open"), ("limit", "50")])
            .send()
            .await?;
        Ok(Self::check(r).await?.json().await?)
    }

    pub async fn review_pull(&self, owner: &str, repo: &str, index: u64, event: ReviewEvent, body: &str) -> Result<()> {
        let r = self
            .req(reqwest::Method::POST, &format!("/repos/{owner}/{repo}/pulls/{index}/reviews"))
            .json(&json!({ "event": event.as_api(), "body": body }))
            .send()
            .await?;
        Self::check(r).await?;
        Ok(())
    }

    /// Faz o merge (`style`: merge, rebase, squash…).
    pub async fn merge_pull(&self, owner: &str, repo: &str, index: u64, style: &str) -> Result<()> {
        let r = self
            .req(reqwest::Method::POST, &format!("/repos/{owner}/{repo}/pulls/{index}/merge"))
            .json(&json!({ "Do": style, "delete_branch_after_merge": true }))
            .send()
            .await?;
        Self::check(r).await?;
        Ok(())
    }
}

/// Divide `dono/repo` em partes.
pub fn split_repo(full: &str) -> Option<(&str, &str)> {
    let (o, r) = full.split_once('/')?;
    (!o.is_empty() && !r.is_empty() && !r.contains('/')).then_some((o, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repo_works() {
        assert_eq!(split_repo("acme/app"), Some(("acme", "app")));
        assert_eq!(split_repo("acme"), None);
        assert_eq!(split_repo("a/b/c"), None);
        assert_eq!(split_repo("/b"), None);
    }

    #[test]
    fn agent_branch() {
        assert_eq!(parse_agent_branch("agent/dev-back/issue-12"), Some(("dev-back", 12)));
        assert_eq!(parse_agent_branch("feature/x"), None);
        assert_eq!(parse_agent_branch("agent/a/b/issue-1"), None);
        assert_eq!(parse_agent_branch("agent/dev/issue-x"), None);
    }

    #[test]
    fn branch_from_ref_strips_refs_heads_prefix() {
        assert_eq!(branch_from_ref("refs/heads/funcionalides"), Some("funcionalides"));
        assert_eq!(branch_from_ref("funcionalides"), Some("funcionalides"));
        assert_eq!(branch_from_ref(""), None);
        assert_eq!(branch_from_ref("   "), None);
    }

    /// Regressão real: a issue tinha um "campo Ramo" (visto em produção via
    /// `GET /issues/{n}`: `"ref":"refs/heads/funcionalides"`) que o orquestrador ignorava
    /// por completo, sempre clonando/abrindo PR contra a branch padrão global.
    #[test]
    fn issue_deserializes_the_branch_field() {
        let issue: Issue =
            serde_json::from_str(r#"{"number":19,"title":"x","ref":"refs/heads/funcionalides","body":"","labels":[]}"#)
                .unwrap();
        assert_eq!(issue.git_ref, "refs/heads/funcionalides");
        assert_eq!(branch_from_ref(&issue.git_ref), Some("funcionalides"));
    }

    #[test]
    fn issue_without_ref_field_defaults_to_empty() {
        let issue: Issue = serde_json::from_str(r#"{"number":1,"title":"x","body":"","labels":[]}"#).unwrap();
        assert_eq!(issue.git_ref, "");
        assert_eq!(branch_from_ref(&issue.git_ref), None);
    }

    #[test]
    fn api_url() {
        let c = ForgejoClient::new("http://forgejo:3000/", "t");
        assert_eq!(c.api_url(), "http://forgejo:3000/api/v1");
    }

    #[test]
    fn pull_parses_minimal() {
        let p: PullRequest = serde_json::from_str(
            r#"{"number":3,"title":"feat","html_url":"u","state":"open","user":{"login":"agent-back"},"head":{"ref":"agent/x"}}"#,
        )
        .unwrap();
        assert_eq!(p.head.unwrap().name, "agent/x");
        assert_eq!(p.user.unwrap().login, "agent-back");
    }
}
