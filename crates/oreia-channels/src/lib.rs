//! Canais humanos. Cada canal recebe mensagens e as entrega ao orquestrador
//! por um `mpsc`, e sabe responder a um usuário ou a todos (broadcast).

pub mod telegram;
pub mod terminal;

use std::sync::Arc;

use async_trait::async_trait;
use oreia_core::{Incoming, Source};
use tokio::sync::mpsc;

pub use telegram::Telegram;
pub use terminal::Terminal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    User(String),
    /// Todos os usuários autorizados do canal.
    Broadcast,
}

impl Recipient {
    pub fn user(id: impl Into<String>) -> Self {
        Recipient::User(id.into())
    }
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &'static str;
    fn source(&self) -> Source;

    async fn send(&self, to: &Recipient, text: &str) -> anyhow::Result<()>;

    /// Indicador de "digitando…", quando o canal suporta.
    async fn typing(&self, _to: &Recipient) {}

    /// Pede aprovação a todos os humanos do canal.
    async fn ask_approval(&self, task_id: &str, text: &str) -> anyhow::Result<()> {
        self.send(&Recipient::Broadcast, &format!("{text}\n\nResponda /approve {task_id} ou /reject {task_id}")).await
    }

    /// Chamado quando a aprovação foi decidida (o Telegram também remove os botões).
    async fn approval_resolved(&self, _task_id: &str, note: &str) {
        let _ = self.send(&Recipient::Broadcast, note).await;
    }

    /// Loop de recebimento. Retorna quando o canal fecha.
    async fn run(self: Arc<Self>, tx: mpsc::Sender<Incoming>) -> anyhow::Result<()>;
}

/// Quebra textos longos em pedaços de até `max` caracteres, preferindo quebras de linha.
pub fn chunk_text(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        if cur.chars().count() + line.chars().count() > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if line.chars().count() > max {
            let chars: Vec<char> = line.chars().collect();
            for piece in chars.chunks(max) {
                out.push(piece.iter().collect());
            }
        } else {
            cur.push_str(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_limit() {
        let t = "a\n".repeat(10);
        let c = chunk_text(&t, 5);
        assert!(c.iter().all(|p| p.chars().count() <= 5));
        assert_eq!(c.concat(), t);

        let long = "x".repeat(12);
        assert_eq!(chunk_text(&long, 5).len(), 3);
        assert_eq!(chunk_text("", 5), vec![String::new()]);
    }
}
