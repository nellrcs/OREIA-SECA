//! Canal de terminal: lê linhas do stdin e imprime as respostas.

use std::sync::Arc;

use async_trait::async_trait;
use oreia_core::{Incoming, Source};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::{Channel, Recipient};

pub const LOCAL_USER: &str = "local";

#[derive(Default)]
pub struct Terminal;

impl Terminal {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Channel for Terminal {
    fn name(&self) -> &'static str {
        "terminal"
    }

    fn source(&self) -> Source {
        Source::Terminal
    }

    async fn send(&self, _to: &Recipient, text: &str) -> anyhow::Result<()> {
        println!("\n🤖 {text}\n");
        Ok(())
    }

    async fn run(self: Arc<Self>, tx: mpsc::Sender<Incoming>) -> anyhow::Result<()> {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        println!("OREIA-SECA pronto. Digite uma tarefa ou /help.");
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "/exit" || line == "/quit" {
                break;
            }
            if tx.send(Incoming::new(Source::Terminal, LOCAL_USER, line)).await.is_err() {
                break;
            }
        }
        tracing::info!("terminal: stdin fechado");
        Ok(())
    }
}
