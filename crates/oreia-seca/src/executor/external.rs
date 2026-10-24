//! Executor externo: roda uma CLI de terceiros (Aider, OpenCode, Goose, Codex,
//! ou qualquer outra) a partir de um template de comando.
//!
//! A instrução da fase vai num arquivo temporário (`{message_file}`), nunca
//! interpolada no shell. A chave do modelo só entra no comando de fato
//! executado — nunca na string de log (`display`), para não vazar em
//! mensagens nem no `data/escalation.jsonl`. Nenhuma outra variável do `.env`
//! chega ao processo: `sandboxed_shell` já roda com ambiente limpo.

use std::path::PathBuf;

use async_trait::async_trait;
use oreia_core::actions::builtin::{run_command, sandboxed_shell};
use oreia_core::actions::ActionContext;

use super::{ModelSelector, PhaseExecutor, PhaseOutcome, PhaseRequest};
use crate::config::env;

/// Presets embutidos. As flags mudam entre versões das ferramentas — confira a
/// documentação atual de cada uma antes de depender destes comandos em produção.
pub const BUILTIN_PRESETS: &[(&str, &str)] = &[
    (
        "aider",
        "aider --model openai/{model} --openai-api-base {base_url} --openai-api-key {api_key} \
         --no-auto-commits --yes-always --message-file {message_file}",
    ),
    ("opencode", "opencode run --model {model} --api-base {base_url} --api-key {api_key} < {message_file}"),
    ("goose", "goose run --instructions {message_file} --provider openai --model {model} --api-base {base_url}"),
    ("codex", "codex exec --model {model} --base-url {base_url} --api-key {api_key} < {message_file}"),
];

pub fn builtin_preset(name: &str) -> Option<&'static str> {
    BUILTIN_PRESETS.iter().find(|(n, _)| *n == name).map(|(_, cmd)| *cmd)
}

pub struct ExternalExecutor {
    name: String,
    template: String,
}

impl ExternalExecutor {
    pub fn new(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self { name: name.into(), template: template.into() }
    }
}

fn render(template: &str, model: &str, base_url: &str, api_key: &str, message_file: &str, test_cmd: &str) -> String {
    template
        .replace("{model}", model)
        .replace("{base_url}", base_url)
        .replace("{api_key}", api_key)
        .replace("{message_file}", message_file)
        .replace("{test_cmd}", test_cmd)
}

fn message_body(req: &PhaseRequest) -> String {
    let mut s = String::new();
    if let Some(agent) = &req.agent_prompt {
        s.push_str("YOUR ROLE:\n");
        s.push_str(agent);
        s.push_str("\n\n");
    }
    s.push_str(&format!(
        "GOAL: {}\n\nCOMPLETED PHASES:\n{}\n\nCURRENT PHASE ({}/{}): {}\n{}",
        req.goal, req.completed_summary, req.phase_index, req.phase_total, req.phase_name, req.phase_instruction
    ));
    if let Some(f) = &req.failure {
        s.push_str(&format!("\n\nPREVIOUS ATTEMPT FAILED: {}\n", f.error));
        if !f.test_output.is_empty() {
            s.push_str(&format!("\nTest output (tail):\n{}\n", f.test_output));
        }
        if !f.diff.is_empty() {
            s.push_str(&format!("\nAttempted diff (truncated):\n{}\n", f.diff));
        }
    }
    s
}

#[async_trait]
impl PhaseExecutor for ExternalExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, req: &PhaseRequest) -> anyhow::Result<PhaseOutcome> {
        let ModelSelector::Pinned { key, spec } = &req.selector else {
            anyhow::bail!("executor externo \"{}\" exige um modelo fixo (degrau de escalada)", self.name);
        };
        let resolved = spec
            .resolve()
            .map_err(|var| anyhow::anyhow!("modelo \"{key}\" não está configurado (falta a variável {var})"))?;

        let dir = req.workdir.join(".oreia_seca");
        tokio::fs::create_dir_all(&dir).await?;
        let message_file: PathBuf = dir.join(format!("phase-{}.md", req.phase_index));
        tokio::fs::write(&message_file, message_body(req)).await?;
        let message_file = message_file.to_string_lossy().into_owned();

        let api_key = spec.api_key_env.as_deref().and_then(env).unwrap_or_default();
        let test_cmd = req.test_cmd.as_deref().unwrap_or("");

        let real = render(&self.template, &resolved.model, &resolved.base_url, &api_key, &message_file, test_cmd);
        let display = render(&self.template, &resolved.model, &resolved.base_url, "***", &message_file, test_cmd);

        let ctx = ActionContext {
            task_id: Some(req.task_id.clone()),
            user_id: String::new(),
            workdir: req.workdir.clone(),
            shell_timeout: req.step_timeout,
        };
        let raw_output = run_command(sandboxed_shell(&real, &ctx), &display, &ctx).await?;

        let changed_files = super::git_changed_files(&req.workdir).await;
        let summary = format!("executor externo \"{}\" ({key}) rodou a fase \"{}\"", self.name, req.phase_name);
        Ok(PhaseOutcome {
            summary,
            changed_files,
            prompt_tokens: 0,
            completion_tokens: 0,
            action_errors: 0,
            raw_output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ModelSelector, PhaseRequest};
    use oreia_llm::ModelSpec;
    use std::time::Duration;

    fn req(workdir: PathBuf, template_test_cmd: Option<&str>) -> PhaseRequest {
        PhaseRequest {
            task_id: "t1".into(),
            workdir,
            goal: "goal".into(),
            completed_summary: "(none)".into(),
            phase_name: "Write".into(),
            phase_instruction: "write it".into(),
            phase_index: 1,
            phase_total: 1,
            agent_prompt: None,
            work_item: None,
            base_branch: "main".into(),
            selector: ModelSelector::Pinned {
                key: "m".into(),
                spec: ModelSpec::local("http://x/v1", "upstream-model"),
            },
            failure: None,
            shell_timeout: Duration::from_secs(5),
            step_timeout: Duration::from_secs(5),
            test_cmd: template_test_cmd.map(String::from),
        }
    }

    #[tokio::test]
    async fn renders_template_writes_message_file_and_hides_secret() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-ext-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        std::env::set_var("OREIA_SECA_TEST_EXT_KEY", "s3cr3t");

        let mut r = req(dir.clone(), Some("cargo test"));
        if let ModelSelector::Pinned { spec, .. } = &mut r.selector {
            spec.api_key_env = Some("OREIA_SECA_TEST_EXT_KEY".into());
        }

        // Script falso: grava os argumentos recebidos para provar o template e o ambiente limpo.
        let exe = ExternalExecutor::new(
            "fake",
            "echo \"model={model} base={base_url} key={api_key} test={test_cmd}\" > out.txt && cat {message_file} >> out.txt && echo \"SECRET=[$OREIA_SECA_TEST_EXT_KEY]\" >> out.txt",
        );
        exe.run(&r).await.unwrap();

        let out = tokio::fs::read_to_string(dir.join("out.txt")).await.unwrap();
        assert!(out.contains("model=upstream-model"), "{out}");
        assert!(out.contains("base=http://x/v1"), "{out}");
        assert!(out.contains("key=s3cr3t"), "{out}");
        assert!(out.contains("test=cargo test"), "{out}");
        assert!(out.contains("CURRENT PHASE (1/1): Write"), "{out}");
        assert!(out.contains("SECRET=[]"), "o segredo do processo pai não deveria chegar ao script: {out}");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[test]
    fn builtin_presets_cover_the_four_tools() {
        for name in ["aider", "opencode", "goose", "codex"] {
            assert!(builtin_preset(name).is_some(), "{name}");
        }
        assert!(builtin_preset("nope").is_none());
    }
}
