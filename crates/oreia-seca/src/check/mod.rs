//! Checagem automática do que mudou na configuração.
//!
//! Diferente de um benchmark: são checagens curtas (timeout de poucos segundos
//! cada), sem rodar tarefas nem a suíte de testes do projeto. Serve para
//! responder "esse `oreia-seca.toml`/`.env` novo funciona?" antes de salvar —
//! usado pelo painel de Configuração do [px0](../../../docs/px0.md), pelo
//! `--check --live` da CLI e pelo `/check` do Telegram.
//!
//! [`diff`] decide SÓ o que mudou (e o que depende disso: um modelo trocado
//! também marca os papéis e degraus de escalada que o usam); [`run`] executa
//! cada alvo em paralelo. Erro de rede (conexão recusada, timeout) nunca é
//! fatal — vira [`CheckStatus::Warn`] ("a máquina pode estar desligada"); só
//! config inválida, token rejeitado, modelo fora da lista ou binário/imagem
//! ausente bloqueiam ([`CheckStatus::Fail`]).

mod basic;
mod executor;
mod forgejo;
pub mod log;
mod model;

use std::collections::BTreeSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            CheckStatus::Ok => "✅",
            CheckStatus::Warn => "⚠️",
            CheckStatus::Fail => "❌",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Identificador estável do alvo (ex. `model:qwen`), igual em toda execução —
    /// útil para comparar duas rodadas (`data/checks.jsonl`).
    pub id: String,
    /// Descrição legível do alvo (ex. `modelo "qwen"`).
    pub target: String,
    pub status: CheckStatus,
    pub message: String,
    pub took_ms: u64,
}

/// O que checar. `diff` produz uma lista deles a partir do que mudou entre
/// duas configs; os comandos "checar tudo" (`--check --live`, `/check`)
/// enumeram todos os alvos da config atual direto, sem diff.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckTarget {
    Model(String),
    Role(String),
    Executor(String),
    Escalation(String),
    Forgejo,
    AgentToken(String),
    Telegram,
    Paths,
    Server,
}

impl CheckTarget {
    pub fn id(&self) -> String {
        match self {
            CheckTarget::Model(k) => format!("model:{k}"),
            CheckTarget::Role(r) => format!("role:{r}"),
            CheckTarget::Executor(e) => format!("executor:{e}"),
            CheckTarget::Escalation(r) => format!("escalation:{r}"),
            CheckTarget::Forgejo => "forgejo".to_string(),
            CheckTarget::AgentToken(a) => format!("agent_token:{a}"),
            CheckTarget::Telegram => "telegram".to_string(),
            CheckTarget::Paths => "paths".to_string(),
            CheckTarget::Server => "server".to_string(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            CheckTarget::Model(k) => format!("modelo \"{k}\""),
            CheckTarget::Role(r) => format!("papel \"{r}\""),
            CheckTarget::Executor(e) => format!("executor \"{e}\""),
            CheckTarget::Escalation(r) => format!("escalada \"{r}\""),
            CheckTarget::Forgejo => "forgejo".to_string(),
            CheckTarget::AgentToken(a) => format!("token do agente \"{a}\""),
            CheckTarget::Telegram => "telegram".to_string(),
            CheckTarget::Paths => "diretórios".to_string(),
            CheckTarget::Server => "servidor".to_string(),
        }
    }
}

/// Todos os alvos da config atual, sem diff — usado por "checar tudo"
/// (`--check --live` sem `--since`, e `/check` no Telegram).
pub fn all_targets(cfg: &Config) -> Vec<CheckTarget> {
    let mut targets = BTreeSet::new();
    for key in cfg.models.keys() {
        targets.insert(CheckTarget::Model(key.clone()));
    }
    for role in cfg.roles.keys() {
        targets.insert(CheckTarget::Role(role.clone()));
    }
    targets.insert(CheckTarget::Executor(cfg.executor.default.clone()));
    for name in cfg.executors.keys() {
        targets.insert(CheckTarget::Executor(name.clone()));
    }
    for (role, esc) in &cfg.escalation {
        targets.insert(CheckTarget::Escalation(role.clone()));
        for step in &esc.steps {
            targets.insert(CheckTarget::Executor(step.executor.clone()));
        }
    }
    if cfg.forgejo.is_some() {
        targets.insert(CheckTarget::Forgejo);
        for a in &cfg.agents {
            targets.insert(CheckTarget::AgentToken(a.name.clone()));
        }
    }
    targets.insert(CheckTarget::Telegram);
    targets.insert(CheckTarget::Paths);
    targets.insert(CheckTarget::Server);
    targets.into_iter().collect()
}

/// Decide o que checar a partir do que mudou entre `old` e `new`. Mudar um
/// modelo também marca os papéis (`[roles]`) e degraus de escalada
/// (`[escalation.*]`) de `new` que o usam — eles dependem do modelo, então uma
/// quebra ali só apareceria indiretamente.
pub fn diff(old: &Config, new: &Config) -> Vec<CheckTarget> {
    let mut targets = BTreeSet::new();

    let model_keys: BTreeSet<&String> = old.models.keys().chain(new.models.keys()).collect();
    let mut changed_models = BTreeSet::new();
    for key in model_keys {
        if old.models.get(key) != new.models.get(key) {
            changed_models.insert(key.clone());
            targets.insert(CheckTarget::Model(key.clone()));
        }
    }

    let role_keys: BTreeSet<&String> = old.roles.keys().chain(new.roles.keys()).collect();
    for role in role_keys {
        let changed_directly = old.roles.get(role) != new.roles.get(role);
        let depends_on_changed_model = new.roles.get(role).into_iter().flatten().any(|m| changed_models.contains(m));
        if changed_directly || depends_on_changed_model {
            targets.insert(CheckTarget::Role(role.clone()));
        }
    }

    if old.executor.default != new.executor.default {
        targets.insert(CheckTarget::Executor(new.executor.default.clone()));
    }
    let executor_keys: BTreeSet<&String> = old.executors.keys().chain(new.executors.keys()).collect();
    for name in executor_keys {
        if old.executors.get(name) != new.executors.get(name) {
            targets.insert(CheckTarget::Executor(name.clone()));
        }
    }

    let escalation_keys: BTreeSet<&String> = old.escalation.keys().chain(new.escalation.keys()).collect();
    for role in escalation_keys {
        let changed_directly = old.escalation.get(role) != new.escalation.get(role);
        let depends_on_changed_model = new
            .escalation
            .get(role)
            .map(|e| e.steps.iter().any(|s| changed_models.contains(&s.model)))
            .unwrap_or(false);
        if changed_directly || depends_on_changed_model {
            targets.insert(CheckTarget::Escalation(role.clone()));
        }
    }

    if old.forgejo != new.forgejo {
        targets.insert(CheckTarget::Forgejo);
    }

    let agent_names: BTreeSet<&String> =
        old.agents.iter().map(|a| &a.name).chain(new.agents.iter().map(|a| &a.name)).collect();
    for name in agent_names {
        let old_agent = old.agents.iter().find(|a| &a.name == name);
        let new_agent = new.agents.iter().find(|a| &a.name == name);
        if old_agent != new_agent {
            targets.insert(CheckTarget::AgentToken(name.clone()));
        }
    }

    if old.telegram != new.telegram {
        targets.insert(CheckTarget::Telegram);
    }
    if old.paths != new.paths {
        targets.insert(CheckTarget::Paths);
    }
    if old.server != new.server {
        targets.insert(CheckTarget::Server);
    }

    targets.into_iter().collect()
}

/// Roda cada alvo em paralelo (são chamadas de rede/processo independentes).
pub async fn run(cfg: &Config, targets: &[CheckTarget]) -> Vec<CheckResult> {
    let mut set = tokio::task::JoinSet::new();
    for (i, target) in targets.iter().cloned().enumerate() {
        let cfg = cfg.clone();
        set.spawn(async move {
            let start = Instant::now();
            let (status, message) = run_one(&cfg, &target).await;
            let result = CheckResult {
                id: target.id(),
                target: target.label(),
                status,
                message,
                took_ms: start.elapsed().as_millis() as u64,
            };
            (i, result)
        });
    }

    let mut out: Vec<(usize, CheckResult)> = Vec::with_capacity(targets.len());
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(pair) => out.push(pair),
            Err(e) => tracing::warn!(error = %e, "checagem de configuração: tarefa falhou"),
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

async fn run_one(cfg: &Config, target: &CheckTarget) -> (CheckStatus, String) {
    match target {
        CheckTarget::Model(key) => model::check_model(cfg, key).await,
        CheckTarget::Role(role) => model::check_role(cfg, role).await,
        CheckTarget::Executor(name) => executor::check_executor(cfg, name).await,
        CheckTarget::Escalation(role) => executor::check_escalation(cfg, role).await,
        CheckTarget::Forgejo => forgejo::check_forgejo(cfg).await,
        CheckTarget::AgentToken(name) => forgejo::check_agent_token(cfg, name).await,
        CheckTarget::Telegram => basic::check_telegram(cfg).await,
        CheckTarget::Paths => basic::check_paths(cfg).await,
        CheckTarget::Server => basic::check_server(cfg).await,
    }
}

/// Mensagem resumida pra canais de texto (Telegram): avisos e falhas por
/// extenso, sucessos só contados — uma config grande e saudável não devia
/// virar uma parede de ✅.
pub fn summarize(results: &[CheckResult]) -> String {
    let ok = results.iter().filter(|r| r.status == CheckStatus::Ok).count();
    let mut lines: Vec<String> = results
        .iter()
        .filter(|r| r.status != CheckStatus::Ok)
        .map(|r| format!("{} {}", r.status.icon(), r.message))
        .collect();
    lines.sort();
    let mut out = Vec::new();
    if ok > 0 {
        out.push(format!("✅ {ok} checagem(ões) ok"));
    }
    out.extend(lines);
    if out.is_empty() {
        "Nada para checar.".to_string()
    } else {
        out.join("\n")
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
    fn diff_on_model_change_marks_the_model_and_its_dependent_roles_and_escalations() {
        let with_escalation =
            format!("{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"builtin\", model = \"qwen\" }}]\n");
        let old = cfg(&with_escalation);
        let new_toml = with_escalation.replace("qwen2.5-coder:14b", "qwen2.5-coder:32b");
        let new = cfg(&new_toml);

        let targets = diff(&old, &new);

        assert!(targets.contains(&CheckTarget::Model("qwen".into())), "{targets:?}");
        // "qwen" é usado em roles.default e roles.direct, e no degrau de dev-back.
        assert!(targets.contains(&CheckTarget::Role("default".into())), "{targets:?}");
        assert!(targets.contains(&CheckTarget::Role("direct".into())), "{targets:?}");
        assert!(targets.contains(&CheckTarget::Escalation("dev-back".into())), "{targets:?}");
        // "llama" não mudou e não depende de "qwen" — não deveria aparecer.
        assert!(!targets.contains(&CheckTarget::Model("llama".into())), "{targets:?}");
        // Nada de Forgejo/Telegram/Paths/Server/AgentToken — nada disso mudou.
        assert!(!targets.contains(&CheckTarget::Forgejo), "{targets:?}");
        assert!(!targets.contains(&CheckTarget::Telegram), "{targets:?}");
        assert!(!targets.contains(&CheckTarget::Paths), "{targets:?}");
        assert!(!targets.contains(&CheckTarget::Server), "{targets:?}");
        assert!(!targets.contains(&CheckTarget::AgentToken("dev-back".into())), "{targets:?}");
    }

    #[test]
    fn diff_is_empty_for_identical_configs() {
        let c = cfg(SAMPLE);
        assert!(diff(&c, &c).is_empty());
    }

    #[test]
    fn diff_detects_agent_token_change() {
        let old = cfg(SAMPLE);
        let new_toml = SAMPLE.replace("AGENT_DEV_BACK_TOKEN", "AGENT_DEV_BACK_TOKEN_2");
        let new = cfg(&new_toml);
        let targets = diff(&old, &new);
        assert_eq!(targets, vec![CheckTarget::AgentToken("dev-back".into())]);
    }

    #[test]
    fn diff_detects_forgejo_change() {
        let old = cfg(SAMPLE);
        let new_toml = SAMPLE.replace("http://forgejo:3000", "http://forgejo:3001");
        let new = cfg(&new_toml);
        assert_eq!(diff(&old, &new), vec![CheckTarget::Forgejo]);
    }

    #[test]
    fn summarize_counts_oks_and_lists_the_rest_by_extenso() {
        let results = vec![
            CheckResult {
                id: "a".into(),
                target: "a".into(),
                status: CheckStatus::Ok,
                message: "a ok".into(),
                took_ms: 1,
            },
            CheckResult {
                id: "b".into(),
                target: "b".into(),
                status: CheckStatus::Ok,
                message: "b ok".into(),
                took_ms: 1,
            },
            CheckResult {
                id: "c".into(),
                target: "c".into(),
                status: CheckStatus::Fail,
                message: "c quebrou".into(),
                took_ms: 1,
            },
        ];
        let text = summarize(&results);
        assert!(text.contains("2 checagem(ões) ok"), "{text}");
        assert!(text.contains("❌ c quebrou"), "{text}");
        assert!(!text.contains("a ok"), "{text}");
    }

    #[test]
    fn all_targets_includes_models_roles_forgejo_and_agents_of_the_sample_config() {
        let c = cfg(SAMPLE);
        let targets = all_targets(&c);
        assert!(targets.contains(&CheckTarget::Model("qwen".into())));
        assert!(targets.contains(&CheckTarget::Model("llama".into())));
        assert!(targets.contains(&CheckTarget::Role("default".into())));
        assert!(targets.contains(&CheckTarget::Forgejo));
        assert!(targets.contains(&CheckTarget::AgentToken("dev-back".into())));
        assert!(targets.contains(&CheckTarget::Telegram));
        assert!(targets.contains(&CheckTarget::Paths));
        assert!(targets.contains(&CheckTarget::Server));
    }
}
