//! Webhooks do Forgejo.
//!
//! O Forgejo assina o corpo com HMAC-SHA256 usando o segredo do webhook e
//! envia o resultado em hex no cabeçalho `X-Forgejo-Signature`
//! (versões antigas e o Gitea usam `X-Gitea-Signature`). O tipo do evento
//! vem em `X-Forgejo-Event` / `X-Gitea-Event`.

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::{Issue, PullRequest, User};

pub const SIGNATURE_HEADERS: &[&str] = &["x-forgejo-signature", "x-gitea-signature"];
pub const EVENT_HEADERS: &[&str] = &["x-forgejo-event", "x-gitea-event"];

/// Confere a assinatura em tempo constante.
pub fn verify_signature(secret: &str, body: &[u8], signature_hex: &str) -> bool {
    let Ok(sig) = hex::decode(signature_hex.trim().trim_start_matches("sha256=")) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// Gera a assinatura (útil em testes e scripts).
pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC aceita qualquer chave");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub owner: User,
    #[serde(default)]
    pub default_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssuesPayload {
    pub action: String,
    pub issue: Issue,
    pub repository: Repository,
    pub sender: User,
    /// Presente em `label_updated`/`labeled`.
    #[serde(default)]
    pub label: Option<crate::Label>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub body: String,
    pub user: User,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssueCommentPayload {
    pub action: String,
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub is_pull: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestPayload {
    pub action: String,
    pub number: u64,
    pub pull_request: PullRequest,
    pub repository: Repository,
    pub sender: User,
}

#[derive(Debug, Clone)]
pub enum Event {
    Issues(IssuesPayload),
    IssueComment(IssueCommentPayload),
    PullRequest(PullRequestPayload),
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("JSON inválido para o evento {event}: {source}")]
    Json { event: String, source: serde_json::Error },
}

pub fn parse_event(event: &str, body: &[u8]) -> Result<Event, ParseError> {
    let wrap = |source| ParseError::Json { event: event.to_string(), source };
    Ok(match event {
        "issues" | "issue_label" => Event::Issues(serde_json::from_slice(body).map_err(wrap)?),
        "issue_comment" | "pull_request_comment" => Event::IssueComment(serde_json::from_slice(body).map_err(wrap)?),
        "pull_request" | "pull_request_sync" | "pull_request_review_approved" | "pull_request_review_rejected" => {
            Event::PullRequest(serde_json::from_slice(body).map_err(wrap)?)
        }
        other => Event::Other(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_roundtrip() {
        let body = br#"{"a":1}"#;
        let sig = sign("segredo", body);
        assert!(verify_signature("segredo", body, &sig));
        assert!(verify_signature("segredo", body, &format!("sha256={sig}")));
        assert!(!verify_signature("outro", body, &sig));
        assert!(!verify_signature("segredo", b"{}", &sig));
        assert!(!verify_signature("segredo", body, "zz"));
    }

    #[test]
    fn parses_issue_labeled() {
        let body = br#"{
          "action":"label_updated",
          "issue":{"number":7,"title":"Login","body":"Fazer login","labels":[{"name":"agent:back"}]},
          "repository":{"name":"app","full_name":"acme/app","owner":{"login":"acme"},"default_branch":"main"},
          "sender":{"login":"warllen"}
        }"#;
        match parse_event("issues", body).unwrap() {
            Event::Issues(p) => {
                assert_eq!(p.issue.number, 7);
                assert_eq!(p.issue.labels[0].name, "agent:back");
                assert_eq!(p.repository.owner.login, "acme");
            }
            e => panic!("evento errado: {e:?}"),
        }
    }

    #[test]
    fn unknown_event_is_other() {
        assert!(matches!(parse_event("push", b"{}").unwrap(), Event::Other(_)));
    }
}
