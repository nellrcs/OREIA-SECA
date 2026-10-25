//! Ações que o modelo pode executar escrevendo tags `<action name="...">`.
//!
//! - [`parser`]: extrai as ações do texto do modelo (formato tolerante, como no original).
//! - [`builtin`]: `shell`, `file_write`, `file_read`, confinadas ao diretório da tarefa.
//! - [`skill`]: skills em Markdown com um bloco ```sh (substituem as skills em JS).

pub mod builtin;
pub mod parser;
pub mod skill;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

pub use parser::{normalize_model_markup, parse_actions, parse_narrative, ParsedAction};

pub type Params = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: String,
    pub description: String,
    pub required: bool,
}

impl ParamSpec {
    pub fn new(name: &str, description: &str, required: bool) -> Self {
        Self { name: name.into(), description: description.into(), required }
    }
}

/// Contexto passado a cada ação.
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub task_id: Option<String>,
    pub user_id: String,
    /// Diretório de trabalho da tarefa. Toda ação de arquivo fica confinada aqui.
    pub workdir: PathBuf,
    /// Timeout de comandos de shell.
    pub shell_timeout: std::time::Duration,
}

#[async_trait]
pub trait Action: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn params(&self) -> Vec<ParamSpec>;
    async fn run(&self, params: &Params, ctx: &ActionContext) -> anyhow::Result<String>;
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActionStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub action: String,
    pub status: ActionStatus,
    pub output: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunResult {
    pub narrative: String,
    pub results: Vec<ActionResult>,
}

impl RunResult {
    pub fn errors(&self) -> usize {
        self.results.iter().filter(|r| r.status == ActionStatus::Error).count()
    }

    /// Bloco de resultados para devolver ao modelo.
    pub fn feedback(&self, max_per_result: usize) -> String {
        self.results
            .iter()
            .map(|r| {
                let status = if r.status == ActionStatus::Ok { "ok" } else { "error" };
                let out: String = r.output.chars().take(max_per_result).collect();
                format!("<result action=\"{}\" status=\"{status}\">\n{out}\n</result>", r.action)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Resumo curto de uma fase (arquivos escritos, comandos rodados).
    pub fn summary(&self) -> String {
        let files: Vec<&str> = self
            .results
            .iter()
            .filter(|r| r.action == "file_write" && r.status == ActionStatus::Ok)
            .filter_map(|r| r.output.strip_prefix("arquivo escrito: "))
            .collect();
        let cmds: Vec<&str> = self
            .results
            .iter()
            .filter(|r| r.action == "shell" && r.status == ActionStatus::Ok)
            .filter_map(|r| r.output.lines().next())
            .map(|l| l.trim_start_matches("$ "))
            .take(2)
            .collect();

        let mut parts = Vec::new();
        if !files.is_empty() {
            parts.push(format!("arquivos: {}", files.join(", ")));
        }
        if !cmds.is_empty() {
            parts.push(format!("cmds: {}", cmds.join(", ")));
        }
        if parts.is_empty() {
            parts.push(self.narrative.lines().next().unwrap_or("").chars().take(120).collect());
        }
        parts.join(" | ").chars().take(200).collect()
    }
}

#[derive(Default, Clone)]
pub struct ActionRegistry {
    handlers: BTreeMap<String, Arc<dyn Action>>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registro com as ações embutidas.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(builtin::ShellAction));
        r.register(Arc::new(builtin::FileWriteAction));
        r.register(Arc::new(builtin::FileReadAction));
        r
    }

    pub fn register(&mut self, action: Arc<dyn Action>) {
        self.handlers.insert(action.name().to_string(), action);
    }

    /// Carrega as skills de `dir` (cada uma em `dir/<nome>/skill.md`). Retorna quantas carregou.
    pub fn load_skills(&mut self, dir: &Path) -> usize {
        let mut n = 0;
        for s in skill::load_dir(dir) {
            tracing::info!(skill = s.name(), "skill carregada");
            self.register(Arc::new(s));
            n += 1;
        }
        n
    }

    pub fn names(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Lista legível para comandos como `/skills`.
    pub fn describe(&self) -> Vec<(String, String, Vec<ParamSpec>)> {
        self.handlers.values().map(|h| (h.name().to_string(), h.description().to_string(), h.params())).collect()
    }

    /// Descrição das ações no formato XML que entra no prompt.
    pub fn prompt_block(&self) -> String {
        self.handlers
            .values()
            .map(|h| {
                let params: String = h
                    .params()
                    .iter()
                    .map(|p| {
                        let req = if p.required { "obrigatório" } else { "opcional" };
                        format!("  <param name=\"{}\">valor <!-- {} ({req}) --></param>\n", p.name, p.description)
                    })
                    .collect();
                format!(
                    "<action name=\"{}\">\n  <!-- {} -->\n{params}</action>",
                    h.name(),
                    h.description().replace('\n', " ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Executa todas as ações encontradas na resposta do modelo, em ordem.
    pub async fn run(&self, model_response: &str, ctx: &ActionContext) -> RunResult {
        let model_response = normalize_model_markup(model_response);
        let actions = parse_actions(&model_response);
        let narrative = parse_narrative(&model_response);
        let mut results = Vec::with_capacity(actions.len());

        for a in actions {
            let Some(handler) = self.handlers.get(&a.name) else {
                results.push(ActionResult {
                    action: a.name.clone(),
                    status: ActionStatus::Error,
                    output: format!("ação desconhecida: \"{}\"", a.name),
                });
                continue;
            };

            if let Some(missing) = handler.params().iter().find(|p| p.required && !a.params.contains_key(&p.name)) {
                results.push(ActionResult {
                    action: a.name.clone(),
                    status: ActionStatus::Error,
                    output: format!("parâmetro obrigatório ausente: {}", missing.name),
                });
                continue;
            }

            tracing::info!(action = %a.name, "executando ação");
            match handler.run(&a.params, ctx).await {
                Ok(output) => results.push(ActionResult { action: a.name, status: ActionStatus::Ok, output }),
                Err(e) => {
                    tracing::warn!(action = %a.name, error = %e, "ação falhou");
                    results.push(ActionResult { action: a.name, status: ActionStatus::Error, output: format!("{e:#}") })
                }
            }
        }
        RunResult { narrative, results }
    }
}

/// Resolve `rel` dentro de `root`, recusando caminhos absolutos e `..`.
pub fn confine(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let rel = rel.trim();
    anyhow::ensure!(!rel.is_empty(), "caminho vazio");
    let p = Path::new(rel);
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => anyhow::bail!("caminho fora da área de trabalho: {rel}"),
        }
    }
    Ok(root.join(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confine_blocks_escapes() {
        let root = Path::new("/w");
        assert_eq!(confine(root, "src/main.rs").unwrap(), PathBuf::from("/w/src/main.rs"));
        assert_eq!(confine(root, "./a.txt").unwrap(), PathBuf::from("/w/./a.txt"));
        assert!(confine(root, "../x").is_err());
        assert!(confine(root, "a/../../x").is_err());
        assert!(confine(root, "/etc/passwd").is_err());
        assert!(confine(root, "").is_err());
    }

    #[tokio::test]
    async fn registry_runs_and_reports() {
        let dir = std::env::temp_dir().join(format!("oreia-reg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = ActionRegistry::with_builtins();
        let ctx = ActionContext {
            task_id: None,
            user_id: "u".into(),
            workdir: dir.clone(),
            shell_timeout: std::time::Duration::from_secs(10),
        };
        let resp = r#"Vou criar o arquivo.
<action name="file_write"><param name="path">hello.txt</param><param name="content">oi</param></action>
<action name="file_read"><param name="path">hello.txt</param></action>
<action name="nao_existe"></action>
<action name="file_read"></action>"#;
        let out = reg.run(resp, &ctx).await;
        assert_eq!(out.narrative, "Vou criar o arquivo.");
        assert_eq!(out.results.len(), 4);
        assert_eq!(out.results[0].status, ActionStatus::Ok);
        assert_eq!(out.results[1].output, "oi");
        assert!(out.results[2].output.contains("desconhecida"));
        assert!(out.results[3].output.contains("obrigatório"));
        assert_eq!(out.errors(), 2);
        assert!(out.summary().contains("hello.txt"));
        assert!(reg.prompt_block().contains("<action name=\"shell\">"));
        std::fs::remove_dir_all(dir).ok();
    }
}
