//! Executor de fases plugável.
//!
//! [`PhaseExecutor`] é a interface que `builtin` (loop agêntico com ações XML,
//! o comportamento original) e `external` (CLI de terceiros como Aider,
//! OpenCode, Goose ou Codex) implementam. A escada de escalada (`maker::exec`)
//! tenta cada degrau nessa interface e usa [`verify`] para decidir se o
//! resultado basta ou se precisa subir de degrau.
//!
//! Regra de projeto: o `ModelRouter` (`oreia-llm`) continua cuidando só de
//! falhas de INFRAESTRUTURA (conexão, timeout, HTTP 5xx/429), com a cadeia de
//! reserva por papel. A escalada aqui trata falhas de QUALIDADE — o modelo
//! respondeu, mas o resultado não passou na verificação. [`run_with_retries`]
//! é a única ponte entre as duas: repete a MESMA tentativa (mesmo degrau,
//! mesmo modelo) quando ela falha por infraestrutura, antes de considerar a
//! fase como "falha de qualidade" e passar a bola para o degrau seguinte.

pub mod builtin;
pub mod external;
pub mod log;
pub mod verify;

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use oreia_core::WorkItem;
use oreia_llm::ModelSpec;

/// Qual modelo usar numa tentativa.
#[derive(Debug, Clone)]
pub enum ModelSelector {
    /// Roteamento por papel, com cadeia de reserva e cooldown — o comportamento atual,
    /// usado pelo degrau único de um papel sem `[escalation.<papel>]`.
    Role(String),
    /// Modelo fixo de um degrau de escalada: não usa a cadeia de reserva do papel,
    /// porque a escolha de qual modelo tentar já é a escalada em si.
    Pinned { key: String, spec: ModelSpec },
}

impl ModelSelector {
    /// Nome para mensagens e registro (chave do modelo ou nome do papel).
    pub fn label(&self) -> &str {
        match self {
            ModelSelector::Role(r) => r,
            ModelSelector::Pinned { key, .. } => key,
        }
    }
}

/// O que deu errado na tentativa anterior desta fase — entra no prompt do
/// próximo degrau como "a tentativa anterior falhou porque...".
#[derive(Debug, Clone, Default)]
pub struct FailureContext {
    pub error: String,
    /// Últimas linhas da saída de `test_cmd`, se configurado.
    pub test_output: String,
    /// Diff (truncado) da tentativa que falhou, antes de ser descartada.
    pub diff: String,
}

#[derive(Debug, Clone)]
pub struct PhaseRequest {
    pub task_id: String,
    pub workdir: PathBuf,
    pub goal: String,
    pub completed_summary: String,
    pub phase_name: String,
    pub phase_instruction: String,
    /// 1-based, para a mensagem "CURRENT PHASE (i/n)".
    pub phase_index: usize,
    pub phase_total: usize,
    pub agent_prompt: Option<String>,
    pub work_item: Option<WorkItem>,
    pub base_branch: String,
    pub selector: ModelSelector,
    pub failure: Option<FailureContext>,
    /// Timeout por comando de shell (ações do executor `builtin`).
    pub shell_timeout: Duration,
    /// Timeout da tentativa inteira (processo do executor `external`).
    pub step_timeout: Duration,
    /// Comando de verificação (`[escalation.<papel>].test_cmd`), repassado como
    /// `{test_cmd}` para executores externos que sabem rodar a própria suíte.
    pub test_cmd: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PhaseOutcome {
    pub summary: String,
    pub changed_files: Vec<String>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Ações com erro durante a tentativa (só o executor `builtin` produz isso).
    pub action_errors: usize,
    pub raw_output: String,
}

#[async_trait]
pub trait PhaseExecutor: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, req: &PhaseRequest) -> anyhow::Result<PhaseOutcome>;
}

/// Repete a MESMA tentativa em falhas de infraestrutura (ver nota no topo do
/// módulo). Falhas de qualidade não entram aqui — isso é a escalada, feita
/// pelo chamador.
pub async fn run_with_retries(
    executor: &dyn PhaseExecutor,
    req: &PhaseRequest,
    retries: usize,
) -> anyhow::Result<PhaseOutcome> {
    let mut attempt = 0;
    loop {
        match executor.run(req).await {
            Ok(r) => return Ok(r),
            Err(e) if attempt < retries => {
                attempt += 1;
                tracing::warn!(attempt, executor = executor.name(), error = %e, "fase falhou, tentando de novo");
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Heurística simples: a instrução (em inglês, como o planner escreve as fases)
/// pede mudança de código? Usado pela verificação para decidir se "nenhum
/// arquivo mudou" é uma falha ou é esperado (ex.: uma fase só de checagem).
pub fn expects_code_change(instruction: &str) -> bool {
    const VERBS: &[&str] = &[
        "write",
        "create",
        "implement",
        "add",
        "fix",
        "refactor",
        "update",
        "edit",
        "modify",
        "delete",
        "remove",
        "rename",
        "install",
        "configure",
        "generate",
        "build",
    ];
    let lower = instruction.to_lowercase();
    VERBS.iter().any(|v| lower.contains(v))
}

/// Arquivos alterados (não commitados) no `workdir`, se for um repositório git.
/// Fora de um repositório git (tarefas sem item do Forgejo), devolve vazio.
pub async fn git_changed_files(workdir: &Path) -> Vec<String> {
    if !workdir.join(".git").exists() {
        return Vec::new();
    }
    let out = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", workdir)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.get(3..))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expects_change_detects_action_verbs() {
        assert!(expects_code_change("Write the soma.py script"));
        assert!(expects_code_change("Fix the failing test in main.rs"));
        assert!(!expects_code_change("Run the test suite and report the result"));
    }

    #[tokio::test]
    async fn git_changed_files_empty_outside_repo() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oreia-seca-nogit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        assert!(git_changed_files(&dir).await.is_empty());
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
