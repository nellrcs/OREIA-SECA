//! Resposta direta (conversa), com até 3 rodadas de ações e resumo
//! automático do histórico quando a janela de contexto enche.

use std::sync::Arc;
use std::time::Duration;

use oreia_core::actions::{parse_narrative, ActionContext};
use oreia_core::session::estimate_tokens;
use oreia_core::{ChatMessage, Incoming};
use oreia_llm::GenerateOpts;

use super::Maker;
use crate::prompts;

const MAX_TOOL_LOOPS: usize = 3;

impl Maker {
    pub(crate) async fn reply_direct(self: Arc<Self>, msg: &Incoming) -> anyhow::Result<()> {
        self.typing(msg.source, &msg.user_id).await;
        let key = msg.session_key();
        let mut history = self.sessions.get(&key);

        let (context, reserve) = self
            .router
            .spec_for("direct")
            .map(|s| (s.context as usize, s.reserve_output as usize))
            .unwrap_or((8_192, 2_048));

        let probe = estimate_tokens(&history) + estimate_tokens(&[ChatMessage::user(&msg.text)]);
        if !history.is_empty() && probe > context.saturating_sub(reserve) {
            match self.summarize_session(&key).await {
                Ok(file) => {
                    history = self.sessions.get(&key);
                    self.reply(msg.source, &msg.user_id, &format!("💾 Contexto comprimido e salvo em {file}")).await;
                }
                Err(e) => tracing::warn!(error = %e, "falha ao resumir contexto"),
            }
        }

        let actions = self.actions();
        let opts = GenerateOpts::system(prompts::direct(&actions.prompt_block())).max_tokens(reserve as u32);
        let workdir = self.cfg.paths.workspace.join("direct").join(sanitize(&key));
        tokio::fs::create_dir_all(&workdir).await?;
        let ctx = ActionContext {
            task_id: None,
            user_id: msg.user_id.clone(),
            workdir,
            shell_timeout: Duration::from_secs(self.cfg.executor.shell_timeout_secs.min(120)),
        };

        let mut messages = history;
        messages.push(ChatMessage::user(&msg.text));

        let mut answer = None;
        for _ in 0..MAX_TOOL_LOOPS {
            let c = self.router.generate("direct", &messages, &opts).await?;
            let run = actions.run(&c.text, &ctx).await;
            if run.results.is_empty() {
                answer = Some(c.text);
                break;
            }
            messages.push(ChatMessage::assistant(c.text));
            messages.push(ChatMessage::user(format!(
                "Resultado da execução:\n{}\n\nAgora responda ao usuário com um resumo claro e objetivo.",
                run.feedback(3_000)
            )));
            self.typing(msg.source, &msg.user_id).await;
        }

        let answer = match answer {
            Some(a) => a,
            None => parse_narrative(&self.router.generate("direct", &messages, &opts).await?.text),
        };

        self.sessions.push(&key, ChatMessage::user(&msg.text));
        self.sessions.push(&key, ChatMessage::assistant(&answer));
        if let Err(e) = self.sessions.flush() {
            tracing::warn!(error = %e, "falha ao salvar sessões");
        }
        self.reply(msg.source, &msg.user_id, &answer).await;
        Ok(())
    }

    /// Resume a sessão com o modelo `direct`, salva em `data/resumos/` e
    /// substitui o histórico pelo resumo. Retorna o nome do arquivo.
    pub(crate) async fn summarize_session(&self, key: &str) -> anyhow::Result<String> {
        let history = self.sessions.get(key);
        anyhow::ensure!(!history.is_empty(), "nenhuma conversa para resumir");

        let transcript: String =
            history.iter().map(|m| format!("{:?}: {}", m.role, m.content)).collect::<Vec<_>>().join("\n\n");
        let c = self
            .router
            .generate(
                "direct",
                &[ChatMessage::user(transcript)],
                &GenerateOpts::system(prompts::SUMMARIZE).max_tokens(1_024),
            )
            .await?;

        let dir = self.cfg.summaries_dir();
        tokio::fs::create_dir_all(&dir).await?;
        let file = format!("{}_{}.md", sanitize(key), chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S"));
        tokio::fs::write(dir.join(&file), &c.text).await?;

        self.sessions.replace(key, vec![ChatMessage::user(format!("📋 Resumo da conversa anterior:\n{}", c.text))]);
        self.sessions.flush()?;
        Ok(file)
    }
}

pub(crate) fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitize_keys() {
        assert_eq!(super::sanitize("telegram:123"), "telegram_123");
        assert_eq!(super::sanitize("../x"), "___x");
    }
}
