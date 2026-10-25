use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// De onde veio uma mensagem ou tarefa.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Terminal,
    Telegram,
    Rest,
    Cron,
    Forgejo,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Terminal => "terminal",
            Source::Telegram => "telegram",
            Source::Rest => "rest",
            Source::Cron => "cron",
            Source::Forgejo => "forgejo",
        }
    }

    /// Canais onde existe um humano lendo em tempo real.
    pub fn is_human(&self) -> bool {
        matches!(self, Source::Terminal | Source::Telegram)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "terminal" => Ok(Source::Terminal),
            "telegram" => Ok(Source::Telegram),
            "rest" => Ok(Source::Rest),
            "cron" => Ok(Source::Cron),
            "forgejo" | "webhook" => Ok(Source::Forgejo),
            other => Err(format!("fonte desconhecida: {other}")),
        }
    }
}

/// O que um agente faz com o `WorkItem`: `Code` escreve e sobe um PR (dev-back,
/// dev-front); `Review` só analisa um PR já aberto por outro agente e comenta —
/// nunca aprova, reprova nem faz merge (isso é sempre comando do humano, via
/// `/merge`/`/changes`, com o token do dono — nunca com o token de um agente).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    #[default]
    Code,
    Review,
}

/// Item de trabalho vindo do Forgejo: qual agente atende qual issue (ou, em
/// modo `Review`, qual PR).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkItem {
    pub owner: String,
    pub repo: String,
    /// Número da issue (`Code`) ou do PR sendo revisado (`Review`) — no
    /// Forgejo/Gitea os dois compartilham a mesma contagem, então nunca colidem.
    pub issue: u64,
    pub agent: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub mode: WorkMode,
    /// Branch a usar no lugar de `branch_name()` — em `Review`, a branch do PR
    /// sendo analisado (que já existe no remoto; não é criada do zero).
    #[serde(default)]
    pub review_branch: Option<String>,
    /// Branch base pra clonar/criar a branch do agente e abrir o PR, no lugar da branch
    /// padrão global (`[forgejo].base_branch`). Em `Code`, vem do campo "Ramo" da issue no
    /// Forgejo, quando preenchido; em `Review`, vem da base real do PR sendo revisado.
    /// `None` = usa a branch padrão global (comportamento de antes deste campo existir).
    #[serde(default)]
    pub base_branch: Option<String>,
}

impl WorkItem {
    pub fn repo_full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn branch_name(&self) -> String {
        format!("agent/{}/issue-{}", self.agent, self.issue)
    }

    /// Branch base pra clonar/abrir PR: `base_branch` quando presente, senão `default`
    /// (a branch padrão global, `[forgejo].base_branch`).
    pub fn effective_base_branch<'a>(&'a self, default: &'a str) -> &'a str {
        self.base_branch.as_deref().unwrap_or(default)
    }

    /// Branch a clonar/fazer checkout: `review_branch` quando presente
    /// (modo `Review`), senão `branch_name()` (modo `Code`, o padrão).
    pub fn checkout_branch(&self) -> String {
        self.review_branch.clone().unwrap_or_else(|| self.branch_name())
    }
}

/// Mensagem recebida por qualquer canal de entrada.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub source: Source,
    pub user_id: String,
    pub text: String,
    pub work_item: Option<WorkItem>,
}

impl Incoming {
    pub fn new(source: Source, user_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { source, user_id: user_id.into(), text: text.into(), work_item: None }
    }

    pub fn with_work_item(mut self, item: WorkItem) -> Self {
        self.work_item = Some(item);
        self
    }

    pub fn session_key(&self) -> String {
        format!("{}:{}", self.source, self.user_id)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Mensagem no formato de chat usado pelos modelos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_roundtrip() {
        for s in [Source::Terminal, Source::Telegram, Source::Rest, Source::Cron, Source::Forgejo] {
            assert_eq!(s.as_str().parse::<Source>().unwrap(), s);
        }
        assert_eq!("webhook".parse::<Source>().unwrap(), Source::Forgejo);
        assert!("x".parse::<Source>().is_err());
    }

    #[test]
    fn work_item_branch() {
        let w = WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 42,
            agent: "dev-back".into(),
            title: String::new(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        };
        assert_eq!(w.branch_name(), "agent/dev-back/issue-42");
        assert_eq!(w.checkout_branch(), w.branch_name(), "modo Code usa branch_name()");
        assert_eq!(w.repo_full_name(), "acme/app");
    }

    #[test]
    fn review_mode_checks_out_the_pr_branch_not_its_own() {
        let w = WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 43,
            agent: "reviewer".into(),
            title: String::new(),
            mode: WorkMode::Review,
            review_branch: Some("agent/dev-back/issue-42".into()),
            base_branch: None,
        };
        // Sem isso, cairia em branch_name() = "agent/reviewer/issue-43", que não existe.
        assert_eq!(w.checkout_branch(), "agent/dev-back/issue-42");
    }

    /// Regressão real: a issue tinha um "campo Ramo" no Forgejo (`refs/heads/funcionalides`)
    /// que a orquestração ignorava, sempre clonando/abrindo PR contra a branch padrão
    /// global — mesmo quando a issue pedia explicitamente outra base.
    #[test]
    fn effective_base_branch_prefers_the_issues_own_branch_field() {
        let mut w = WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 1,
            agent: "dev-back".into(),
            title: String::new(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        };
        assert_eq!(w.effective_base_branch("main"), "main");

        w.base_branch = Some("funcionalides".into());
        assert_eq!(w.effective_base_branch("main"), "funcionalides");
    }
}
