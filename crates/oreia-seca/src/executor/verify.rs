//! Verificação de uma tentativa de fase: decide se o resultado basta ou se a
//! fase precisa escalar para o próximo degrau.
//!
//! A fase FALHA se: (1) o executor retornou erro ou estourou o tempo; (2)
//! nenhum arquivo mudou e a instrução pedia mudança de código; (3) `test_cmd`
//! existe e terminou com código != 0. Sem `test_cmd`, só (1) e (2) valem (modo
//! fraco). O que o modelo *diz* que fez nunca entra nessa decisão.

use std::path::Path;
use std::time::Duration;

use oreia_core::actions::builtin::{run_command, sandboxed_shell};
use oreia_core::actions::ActionContext;

use super::{expects_code_change, FailureContext, PhaseOutcome};

const TEST_TAIL_LINES: usize = 150;
const DIFF_MAX_CHARS: usize = 4_000;

#[derive(Debug, Clone)]
pub enum Verdict {
    Passed,
    /// `true` quando nem (1) nem (2)/(3) foram checados por falta de `test_cmd` —
    /// usado para avisar que a escalada está em modo fraco.
    Failed(FailureContext),
}

pub async fn verify(
    workdir: &Path,
    instruction: &str,
    exec_result: &anyhow::Result<PhaseOutcome>,
    test_cmd: Option<&str>,
    shell_timeout: Duration,
) -> Verdict {
    let outcome = match exec_result {
        Err(e) => {
            return Verdict::Failed(FailureContext {
                error: format!("o executor falhou: {e:#}"),
                test_output: String::new(),
                diff: String::new(),
            })
        }
        Ok(o) => o,
    };

    if outcome.changed_files.is_empty() && expects_code_change(instruction) {
        return Verdict::Failed(FailureContext {
            error: "nenhum arquivo foi alterado, mas a fase pedia mudança de código".into(),
            test_output: outcome.raw_output.chars().take(500).collect(),
            diff: String::new(),
        });
    }

    if let Some(cmd) = test_cmd {
        let ctx =
            ActionContext { task_id: None, user_id: String::new(), workdir: workdir.to_path_buf(), shell_timeout };
        if let Err(e) = run_command(sandboxed_shell(cmd, &ctx), cmd, &ctx).await {
            let full = e.to_string();
            let tail: Vec<&str> = full.lines().collect();
            let tail = tail[tail.len().saturating_sub(TEST_TAIL_LINES)..].join("\n");
            let diff = git_diff_truncated(workdir).await;
            return Verdict::Failed(FailureContext {
                error: format!("test_cmd falhou: {cmd}"),
                test_output: tail,
                diff,
            });
        }
    }

    Verdict::Passed
}

async fn git_diff_truncated(workdir: &Path) -> String {
    if !workdir.join(".git").exists() {
        return String::new();
    }
    let out = tokio::process::Command::new("git")
        .arg("diff")
        .current_dir(workdir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", workdir)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).chars().take(DIFF_MAX_CHARS).collect(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "oreia-seca-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ))
    }

    #[tokio::test]
    async fn fails_when_executor_errored() {
        let d = dir();
        tokio::fs::create_dir_all(&d).await.unwrap();
        let err: anyhow::Result<PhaseOutcome> = Err(anyhow::anyhow!("boom"));
        let v = verify(&d, "write x", &err, None, Duration::from_secs(5)).await;
        assert!(matches!(v, Verdict::Failed(f) if f.error.contains("boom")));
        tokio::fs::remove_dir_all(&d).await.ok();
    }

    #[tokio::test]
    async fn fails_when_no_change_and_instruction_expects_code() {
        let d = dir();
        tokio::fs::create_dir_all(&d).await.unwrap();
        let ok = Ok(PhaseOutcome { changed_files: vec![], ..Default::default() });
        let v = verify(&d, "Write the soma.py script", &ok, None, Duration::from_secs(5)).await;
        assert!(matches!(v, Verdict::Failed(_)));
        tokio::fs::remove_dir_all(&d).await.ok();
    }

    #[tokio::test]
    async fn passes_without_test_cmd_when_change_matches_instruction() {
        let d = dir();
        tokio::fs::create_dir_all(&d).await.unwrap();
        let ok = Ok(PhaseOutcome { changed_files: vec!["soma.py".into()], ..Default::default() });
        let v = verify(&d, "Write the soma.py script", &ok, None, Duration::from_secs(5)).await;
        assert!(matches!(v, Verdict::Passed));
        tokio::fs::remove_dir_all(&d).await.ok();
    }

    #[tokio::test]
    async fn runs_test_cmd_and_reports_failure_output() {
        let d = dir();
        tokio::fs::create_dir_all(&d).await.unwrap();
        let ok = Ok(PhaseOutcome { changed_files: vec!["x".into()], ..Default::default() });
        let v = verify(&d, "write x", &ok, Some("echo falhou-no-teste >&2; exit 1"), Duration::from_secs(5)).await;
        match v {
            Verdict::Failed(f) => assert!(f.test_output.contains("falhou-no-teste"), "{}", f.test_output),
            Verdict::Passed => panic!("deveria falhar"),
        }
        tokio::fs::remove_dir_all(&d).await.ok();
    }

    #[tokio::test]
    async fn passes_when_test_cmd_succeeds() {
        let d = dir();
        tokio::fs::create_dir_all(&d).await.unwrap();
        let ok = Ok(PhaseOutcome { changed_files: vec!["x".into()], ..Default::default() });
        let v = verify(&d, "write x", &ok, Some("true"), Duration::from_secs(5)).await;
        assert!(matches!(v, Verdict::Passed));
        tokio::fs::remove_dir_all(&d).await.ok();
    }
}
