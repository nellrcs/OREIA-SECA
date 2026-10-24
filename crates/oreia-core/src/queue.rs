//! Fila de tarefas com concorrência limitada e timeout por tarefa.
//! Equivalente ao `TaskQueue` do OREIA-SECA, sobre um semáforo do Tokio.

use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("{0:#}")]
    Task(anyhow::Error),
    #[error("timeout: \"{label}\" excedeu {secs}s")]
    Timeout { label: String, secs: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct QueueStatus {
    pub running: usize,
    pub queued: usize,
    pub concurrency: usize,
    pub completed: u64,
    pub failed: u64,
}

#[derive(Default)]
struct Counters {
    running: AtomicUsize,
    queued: AtomicUsize,
    completed: AtomicU64,
    failed: AtomicU64,
}

#[derive(Clone)]
pub struct TaskQueue {
    sem: Arc<Semaphore>,
    concurrency: usize,
    timeout: Option<Duration>,
    counters: Arc<Counters>,
}

impl TaskQueue {
    pub fn new(concurrency: usize, timeout: Option<Duration>) -> Self {
        let concurrency = concurrency.max(1);
        Self {
            sem: Arc::new(Semaphore::new(concurrency)),
            concurrency,
            timeout,
            counters: Arc::new(Counters::default()),
        }
    }

    /// Enfileira uma tarefa. Ela começa assim que houver vaga.
    pub fn spawn<T, Fut>(&self, label: impl Into<String>, fut: Fut) -> JoinHandle<Result<T, QueueError>>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let label = label.into();
        let sem = self.sem.clone();
        let c = self.counters.clone();
        let timeout = self.timeout;

        c.queued.fetch_add(1, Ordering::SeqCst);
        tracing::debug!(%label, "tarefa enfileirada");

        tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semáforo da fila fechado");
            c.queued.fetch_sub(1, Ordering::SeqCst);
            c.running.fetch_add(1, Ordering::SeqCst);
            tracing::info!(%label, "iniciando tarefa");

            let result = match timeout {
                Some(d) => match tokio::time::timeout(d, fut).await {
                    Ok(r) => r.map_err(QueueError::Task),
                    Err(_) => Err(QueueError::Timeout { label: label.clone(), secs: d.as_secs() }),
                },
                None => fut.await.map_err(QueueError::Task),
            };

            c.running.fetch_sub(1, Ordering::SeqCst);
            match &result {
                Ok(_) => c.completed.fetch_add(1, Ordering::SeqCst),
                Err(e) => {
                    tracing::warn!(%label, error = %e, "tarefa falhou");
                    c.failed.fetch_add(1, Ordering::SeqCst)
                }
            };
            result
        })
    }

    pub fn status(&self) -> QueueStatus {
        QueueStatus {
            running: self.counters.running.load(Ordering::SeqCst),
            queued: self.counters.queued.load(Ordering::SeqCst),
            concurrency: self.concurrency,
            completed: self.counters.completed.load(Ordering::SeqCst),
            failed: self.counters.failed.load(Ordering::SeqCst),
        }
    }

    pub fn is_idle(&self) -> bool {
        let s = self.status();
        s.running == 0 && s.queued == 0
    }

    /// Espera as tarefas em andamento terminarem (até `max`). Usado no shutdown.
    pub async fn drain(&self, max: Duration) -> bool {
        tokio::time::timeout(max, async {
            while !self.is_idle() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn respects_concurrency() {
        let q = TaskQueue::new(1, None);
        let peak = Arc::new(AtomicUsize::new(0));
        let now = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for i in 0..4 {
            let (peak, now) = (peak.clone(), now.clone());
            handles.push(q.spawn(format!("t{i}"), async move {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                now.fetch_sub(1, Ordering::SeqCst);
                Ok(i)
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        let s = q.status();
        assert_eq!((s.completed, s.failed, s.running, s.queued), (4, 0, 0, 0));
    }

    #[tokio::test]
    async fn timeout_counts_as_failure() {
        let q = TaskQueue::new(2, Some(Duration::from_millis(20)));
        let h = q.spawn("lenta", async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        });
        let err = h.await.unwrap().unwrap_err();
        assert!(matches!(err, QueueError::Timeout { .. }));
        assert_eq!(q.status().failed, 1);
    }

    #[tokio::test]
    async fn drain_waits_for_work() {
        let q = TaskQueue::new(1, None);
        let _h = q.spawn("x", async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(())
        });
        assert!(q.drain(Duration::from_secs(2)).await);
        assert!(q.is_idle());
    }
}
