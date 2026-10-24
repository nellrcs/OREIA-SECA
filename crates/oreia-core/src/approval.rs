//! Aprovação humana cross-canal (Human-in-the-Loop).
//!
//! 1. Uma fonte automática (REST, cron) gera uma tarefa.
//! 2. O orquestrador chama [`ApprovalBroker::request`] e avisa os canais humanos.
//! 3. Alguém responde `/approve <id>` ou `/reject <id>` em qualquer canal.
//! 4. [`ApprovalBroker::respond`] libera a espera.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Approved,
    Rejected,
    Expired,
}

struct Pending {
    tx: oneshot::Sender<bool>,
    question: String,
}

#[derive(Default)]
pub struct ApprovalBroker {
    pending: Mutex<BTreeMap<String, Pending>>,
}

impl ApprovalBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra o pedido e espera a resposta (ou o timeout).
    pub async fn request(&self, id: &str, question: &str, timeout: Duration) -> ApprovalOutcome {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("lock de aprovações envenenado")
            .insert(id.to_string(), Pending { tx, question: question.to_string() });

        tracing::info!(id, "aguardando aprovação");
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(true)) => ApprovalOutcome::Approved,
            Ok(Ok(false)) | Ok(Err(_)) => ApprovalOutcome::Rejected,
            Err(_) => {
                self.pending.lock().expect("lock de aprovações envenenado").remove(id);
                tracing::warn!(id, "aprovação expirou sem resposta");
                ApprovalOutcome::Expired
            }
        }
    }

    /// Responde a um pedido pendente. Retorna `false` se não havia pedido com esse id.
    pub fn respond(&self, id: &str, approved: bool) -> bool {
        let entry = self.pending.lock().expect("lock de aprovações envenenado").remove(id);
        match entry {
            Some(p) => {
                let _ = p.tx.send(approved);
                tracing::info!(id, approved, "aprovação respondida");
                true
            }
            None => false,
        }
    }

    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.lock().expect("lock de aprovações envenenado").contains_key(id)
    }

    /// Lista `(id, resumo da pergunta)` dos pedidos pendentes.
    pub fn list_pending(&self) -> Vec<(String, String)> {
        self.pending
            .lock()
            .expect("lock de aprovações envenenado")
            .iter()
            .map(|(id, p)| (id.clone(), p.question.chars().take(80).collect()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn approve_flow() {
        let b = Arc::new(ApprovalBroker::new());
        let b2 = b.clone();
        let h = tokio::spawn(async move { b2.request("t1", "plano", Duration::from_secs(5)).await });
        while !b.is_pending("t1") {
            tokio::task::yield_now().await;
        }
        assert_eq!(b.list_pending().len(), 1);
        assert!(b.respond("t1", true));
        assert_eq!(h.await.unwrap(), ApprovalOutcome::Approved);
        assert!(!b.respond("t1", true), "não pode responder duas vezes");
    }

    #[tokio::test]
    async fn reject_flow() {
        let b = Arc::new(ApprovalBroker::new());
        let b2 = b.clone();
        let h = tokio::spawn(async move { b2.request("t2", "x", Duration::from_secs(5)).await });
        while !b.is_pending("t2") {
            tokio::task::yield_now().await;
        }
        b.respond("t2", false);
        assert_eq!(h.await.unwrap(), ApprovalOutcome::Rejected);
    }

    #[tokio::test]
    async fn expires() {
        let b = ApprovalBroker::new();
        let out = b.request("t3", "x", Duration::from_millis(10)).await;
        assert_eq!(out, ApprovalOutcome::Expired);
        assert!(!b.is_pending("t3"));
    }
}
