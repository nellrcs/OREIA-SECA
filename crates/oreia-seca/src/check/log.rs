//! Histórico de checagens em `data/checks.jsonl` — uma linha por resultado,
//! com o timestamp e o que mudou. Mesmo formato de `executor::log`
//! (`data/escalation.jsonl`): fácil de inspecionar e fazer backup.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::CheckResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckLogEntry {
    pub id: String,
    pub target: String,
    pub status: super::CheckStatus,
    pub message: String,
    pub took_ms: u64,
    pub at: chrono::DateTime<chrono::Utc>,
}

impl CheckLogEntry {
    pub fn from_result(r: &CheckResult) -> Self {
        Self {
            id: r.id.clone(),
            target: r.target.clone(),
            status: r.status,
            message: r.message.clone(),
            took_ms: r.took_ms,
            at: chrono::Utc::now(),
        }
    }
}

pub async fn append(path: &Path, results: &[CheckResult]) -> anyhow::Result<()> {
    if results.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut buf = String::new();
    for r in results {
        buf.push_str(&serde_json::to_string(&CheckLogEntry::from_result(r))?);
        buf.push('\n');
    }
    let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
    f.write_all(buf.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{CheckStatus, CheckTarget};

    fn result(status: CheckStatus) -> CheckResult {
        CheckResult {
            id: CheckTarget::Telegram.id(),
            target: CheckTarget::Telegram.label(),
            status,
            message: "telegram: token ok".into(),
            took_ms: 42,
        }
    }

    #[tokio::test]
    async fn appends_one_json_line_per_result_and_keeps_earlier_lines() {
        let path = std::env::temp_dir().join(format!("oreia-seca-checks-{}.jsonl", std::process::id()));
        tokio::fs::remove_file(&path).await.ok();

        append(&path, &[result(CheckStatus::Ok)]).await.unwrap();
        append(&path, &[result(CheckStatus::Fail)]).await.unwrap();

        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2, "{text}");
        let parsed: Vec<CheckLogEntry> = lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(parsed[0].status, CheckStatus::Ok);
        assert_eq!(parsed[1].status, CheckStatus::Fail);

        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn append_with_no_results_does_not_create_the_file() {
        let path = std::env::temp_dir().join(format!("oreia-seca-checks-empty-{}.jsonl", std::process::id()));
        tokio::fs::remove_file(&path).await.ok();
        append(&path, &[]).await.unwrap();
        assert!(!path.exists());
    }
}
