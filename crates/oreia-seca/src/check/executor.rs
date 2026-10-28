//! Checagem de `[executors.*]`/presets embutidos e de `[escalation.*]`.
//!
//! Não executa a ferramenta de verdade (isso é o que a tarefa faz) — só
//! confere que o template renderiza sem sobrar placeholder e que o binário
//! (ou a imagem Docker, para um executor containerizado) existe.

use std::time::Duration;

use super::CheckStatus;
use crate::config::Config;
use crate::executor::external::builtin_preset;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve o template de um executor: `[executors.<nome>].command` quando
/// presente, senão o preset embutido do mesmo nome — mesma regra de
/// `Config::validate_executor_name`/`ExternalExecutor`.
fn resolve_template(cfg: &Config, name: &str) -> Result<(String, Option<String>), String> {
    match cfg.executors.get(name) {
        Some(preset) => {
            let template = match &preset.command {
                Some(c) => c.clone(),
                None => builtin_preset(name)
                    .map(str::to_string)
                    .ok_or_else(|| format!("[executors.{name}] não tem \"command\" nem é um preset embutido"))?,
            };
            Ok((template, preset.container.clone()))
        }
        None => builtin_preset(name)
            .map(|c| (c.to_string(), None))
            .ok_or_else(|| format!("\"{name}\" não está em [executors] nem é um preset embutido")),
    }
}

/// Renderiza com valores fictícios e procura um `{palavra}` que não seja um
/// dos nossos placeholders conhecidos — pega template com typo (ex.
/// `{modle}`) antes de uma tarefa de verdade tropeçar nele. Não confunde com
/// `${VAR%...}` (sintaxe de shell do preset do goose): só conta chaves cujo
/// caractere anterior não é `$`.
fn leftover_placeholder(template: &str) -> Option<String> {
    let rendered = template
        .replace("{model}", "dummy-model")
        .replace("{base_url}", "http://dummy.invalid/v1")
        .replace("{api_key}", "dummy-key")
        .replace("{message_file}", "/tmp/dummy-message.md")
        .replace("{test_cmd}", "true")
        .replace("{config_dir}", "/tmp/dummy-config-dir");

    let chars: Vec<char> = rendered.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '{' || (i > 0 && chars[i - 1] == '$') {
            continue;
        }
        let Some(rel_end) = chars[i + 1..].iter().position(|&c| c == '}') else { continue };
        let word: String = chars[i + 1..i + 1 + rel_end].iter().collect();
        if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Some(format!("{{{word}}}"));
        }
    }
    None
}

/// Primeiro token do template que não é uma atribuição de variável de
/// ambiente (`VAR=valor`) nem um placeholder ainda não substituído — o
/// binário de verdade que o preset chama. Heurística: cobre os quatro
/// presets embutidos e qualquer `[executors.*]` no mesmo estilo (ver
/// `docs/executores.md`); não tenta entender shell de verdade.
fn first_binary(template: &str) -> Option<&str> {
    template
        .split_whitespace()
        .map(|tok| tok.trim_matches(';'))
        .find(|tok| !tok.is_empty() && !tok.starts_with('{') && !tok.contains('='))
}

async fn check_binary(name: &str, template: &str) -> (CheckStatus, String) {
    let Some(bin) = first_binary(template) else {
        return (
            CheckStatus::Fail,
            format!("executor \"{name}\": não consegui identificar o binário no template de comando"),
        );
    };
    let run = tokio::process::Command::new(bin).arg("--version").kill_on_drop(true).output();
    match tokio::time::timeout(TIMEOUT, run).await {
        Ok(Ok(_)) => (CheckStatus::Ok, format!("executor \"{name}\": binário \"{bin}\" encontrado no PATH")),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            (CheckStatus::Fail, format!("executor \"{name}\": binário \"{bin}\" não encontrado no PATH"))
        }
        Ok(Err(e)) => (CheckStatus::Warn, format!("executor \"{name}\": falha ao checar \"{bin}\": {e}")),
        Err(_) => (CheckStatus::Warn, format!("executor \"{name}\": \"{bin} --version\" não respondeu em 10s")),
    }
}

async fn check_docker_image(name: &str, image: &str) -> (CheckStatus, String) {
    let run = tokio::process::Command::new("docker").args(["image", "inspect", image]).output();
    match tokio::time::timeout(TIMEOUT, run).await {
        Ok(Ok(out)) if out.status.success() => {
            (CheckStatus::Ok, format!("executor \"{name}\": imagem \"{image}\" presente"))
        }
        Ok(Ok(_)) => (
            CheckStatus::Fail,
            format!("executor \"{name}\": imagem \"{image}\" não encontrada (rode \"make executors-build\")"),
        ),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            (CheckStatus::Warn, format!("executor \"{name}\": \"docker\" não encontrado no PATH deste host"))
        }
        Ok(Err(e)) => (CheckStatus::Warn, format!("executor \"{name}\": falha ao chamar docker: {e}")),
        Err(_) => (CheckStatus::Warn, format!("executor \"{name}\": \"docker image inspect\" não respondeu em 10s")),
    }
}

pub async fn check_executor(cfg: &Config, name: &str) -> (CheckStatus, String) {
    if name == "builtin" {
        return (CheckStatus::Ok, "executor \"builtin\": embutido, sempre disponível".to_string());
    }

    let (template, container) = match resolve_template(cfg, name) {
        Ok(v) => v,
        Err(msg) => return (CheckStatus::Fail, format!("executor \"{name}\": {msg}")),
    };

    if let Some(bad) = leftover_placeholder(&template) {
        return (CheckStatus::Fail, format!("executor \"{name}\": template com placeholder desconhecido: {bad}"));
    }

    match container {
        Some(image) => check_docker_image(name, &image).await,
        None => check_binary(name, &template).await,
    }
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub async fn check_escalation(cfg: &Config, role: &str) -> (CheckStatus, String) {
    let Some(esc) = cfg.escalation.get(role) else {
        return (CheckStatus::Fail, format!("escalada \"{role}\" não existe mais em [escalation]"));
    };

    let Some(test_cmd) = esc.test_cmd.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return (CheckStatus::Ok, format!("escalada \"{role}\": {} degrau(s), sem test_cmd", esc.steps.len()));
    };
    let Some(bin) = test_cmd.split_whitespace().next() else {
        return (CheckStatus::Ok, format!("escalada \"{role}\": {} degrau(s)", esc.steps.len()));
    };

    let run =
        tokio::process::Command::new("sh").arg("-c").arg(format!("command -v {}", shell_single_quote(bin))).output();
    match tokio::time::timeout(TIMEOUT, run).await {
        Ok(Ok(out)) if out.status.success() => {
            (CheckStatus::Ok, format!("escalada \"{role}\": test_cmd \"{bin}\" encontrado no PATH"))
        }
        Ok(Ok(_)) => (CheckStatus::Fail, format!("escalada \"{role}\": test_cmd \"{bin}\" não encontrado no PATH")),
        Ok(Err(e)) => (CheckStatus::Warn, format!("escalada \"{role}\": falha ao checar test_cmd: {e}")),
        Err(_) => (CheckStatus::Warn, format!("escalada \"{role}\": checagem do test_cmd não respondeu em 10s")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::SAMPLE;

    fn cfg(toml: &str) -> Config {
        let c: Config = toml::from_str(toml).unwrap();
        c.validate().unwrap();
        c
    }

    #[test]
    fn leftover_placeholder_catches_unknown_token_but_not_shell_dollar_syntax() {
        assert_eq!(leftover_placeholder("echo {modle}"), Some("{modle}".to_string()));
        assert_eq!(leftover_placeholder("OPENAI_HOST=\"${BASE_URL%/v1}\" goose run"), None);
        for name in ["aider", "opencode", "goose", "codex"] {
            assert_eq!(leftover_placeholder(builtin_preset(name).unwrap()), None, "{name}");
        }
    }

    #[test]
    fn first_binary_skips_env_assignments_for_all_builtin_presets() {
        assert_eq!(first_binary(builtin_preset("aider").unwrap()), Some("aider"));
        assert_eq!(first_binary(builtin_preset("opencode").unwrap()), Some("opencode"));
        assert_eq!(first_binary(builtin_preset("goose").unwrap()), Some("goose"));
        assert_eq!(first_binary(builtin_preset("codex").unwrap()), Some("codex"));
    }

    #[tokio::test]
    async fn builtin_executor_is_always_ok() {
        let c = cfg(SAMPLE);
        let (status, _) = check_executor(&c, "builtin").await;
        assert_eq!(status, CheckStatus::Ok);
    }

    #[tokio::test]
    async fn unknown_template_placeholder_fails() {
        let toml = format!("{SAMPLE}\n[executors.minha-cli]\ncommand = \"minha-cli {{modle}}\"\n");
        let c = cfg(&toml);
        let (status, msg) = check_executor(&c, "minha-cli").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
        assert!(msg.contains("{modle}"), "{msg}");
    }

    #[tokio::test]
    async fn missing_binary_fails() {
        let toml = format!(
            "{SAMPLE}\n[executors.minha-cli]\ncommand = \"oreia-seca-definitely-not-a-real-binary-xyz {{message_file}}\"\n"
        );
        let c = cfg(&toml);
        let (status, msg) = check_executor(&c, "minha-cli").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
    }

    #[tokio::test]
    async fn escalation_without_test_cmd_is_ok() {
        let toml =
            format!("{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"builtin\", model = \"qwen\" }}]\n");
        let c = cfg(&toml);
        let (status, msg) = check_escalation(&c, "dev-back").await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");
    }

    #[tokio::test]
    async fn escalation_with_missing_test_cmd_binary_fails() {
        let toml = format!(
            "{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"builtin\", model = \"qwen\" }}]\ntest_cmd = \"oreia-seca-definitely-not-a-real-binary-xyz\"\n"
        );
        let c = cfg(&toml);
        let (status, msg) = check_escalation(&c, "dev-back").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
    }
}
