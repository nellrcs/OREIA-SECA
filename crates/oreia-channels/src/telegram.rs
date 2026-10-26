//! Bot do Telegram via Bot API com long polling (sem dependência de framework).
//!
//! Segurança: só IDs em `allowed` conversam com o bot. Se a lista estiver
//! vazia, ninguém é aceito; o bot responde informando o ID do remetente para
//! facilitar a configuração.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use oreia_core::{Incoming, Source};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::{chunk_text, Channel, Recipient};

const CHUNK: usize = 4000;

/// Comandos deste bot, pra registrar no Telegram via `setMyCommands` — sem isso,
/// o menu "/" do app mostra o que ficou registrado por QUALQUER OUTRA aplicação
/// que já usou esse mesmo token (Telegram guarda isso por bot, não por app que
/// está rodando), o que parece "lixo" pra quem reaproveitou um bot antigo.
/// Mantenha em sincronia com `oreia_seca::maker::commands::HELP`.
const BOT_COMMANDS: &[(&str, &str)] = &[
    ("status", "Fila, aprovações e modelos ativos"),
    ("queue", "Detalhes da fila"),
    ("tasks", "Suas últimas tarefas"),
    ("feature", "Cria uma tarefa sem passar pelo classificador"),
    ("issue", "Abre uma issue no Forgejo (passo a passo se vier sem nada)"),
    ("assign", "Liga um agente a uma issue já aberta"),
    ("pending", "Aprovações pendentes"),
    ("approve", "Aprova uma tarefa pendente"),
    ("reject", "Rejeita uma tarefa pendente"),
    ("retry", "Retoma uma tarefa interrompida"),
    ("models", "Modelo ativo por papel"),
    ("stats", "Estatísticas de escalada (degraus, custo)"),
    ("agents", "Agentes do Forgejo"),
    ("prs", "PRs abertos, um cartão por PR"),
    ("merge", "Aprova e integra um PR"),
    ("changes", "Pede ajustes num PR"),
    ("skills", "Ações disponíveis"),
    ("resumir", "Resume e compacta esta conversa"),
    ("limpar", "Apaga o histórico desta conversa"),
    ("help", "Lista de comandos"),
];

pub struct Telegram {
    http: reqwest::Client,
    api: String,
    allowed: Vec<i64>,
    /// task_id → mensagens com botões de aprovação (chat_id, message_id)
    approval_msgs: Mutex<HashMap<String, Vec<(i64, i64)>>>,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    callback_query: Option<CallbackQuery>,
}

#[derive(Deserialize)]
struct Message {
    message_id: i64,
    chat: Chat,
    #[serde(default)]
    from: Option<From>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Deserialize)]
struct From {
    id: i64,
}

#[derive(Deserialize)]
struct CallbackQuery {
    id: String,
    from: From,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    message: Option<Message>,
}

impl Telegram {
    pub fn new(token: &str, allowed: Vec<i64>) -> Self {
        Self::with_api(&format!("https://api.telegram.org/bot{token}"), allowed)
    }

    /// Permite apontar para um servidor de testes.
    pub fn with_api(api: &str, allowed: Vec<i64>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api: api.trim_end_matches('/').to_string(),
            allowed,
            approval_msgs: Mutex::new(HashMap::new()),
        }
    }

    /// Lê `"123, 456"` em uma lista de IDs.
    pub fn parse_allowed(raw: &str) -> Vec<i64> {
        raw.split(',').filter_map(|s| s.trim().parse().ok()).collect()
    }

    fn is_allowed(&self, user_id: i64) -> bool {
        self.allowed.contains(&user_id)
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: Value,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let resp: ApiResponse<T> =
            self.http.post(format!("{}/{method}", self.api)).timeout(timeout).json(&body).send().await?.json().await?;
        if !resp.ok {
            anyhow::bail!("Telegram {method}: {}", resp.description.unwrap_or_default());
        }
        resp.result.ok_or_else(|| anyhow::anyhow!("Telegram {method}: resposta sem result"))
    }

    /// Repete `call` em falha de transporte (timeout, conexão recusada) com
    /// uma pequena espera entre tentativas — uma rede instável não deveria
    /// simplesmente apagar a mensagem, como já aconteceu num teste real deste projeto.
    async fn call_with_retries<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: Value,
        timeout: Duration,
        attempts: usize,
    ) -> anyhow::Result<T> {
        let mut last_err = None;
        for attempt in 0..attempts.max(1) {
            match self.call(method, body.clone(), timeout).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    tracing::warn!(method, attempt, error = %e, "telegram: chamada falhou, tentando de novo");
                    last_err = Some(e);
                    if attempt + 1 < attempts {
                        tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt as u32))).await;
                    }
                }
            }
        }
        Err(last_err.expect("attempts >= 1 garante pelo menos uma tentativa"))
    }

    async fn send_to(&self, chat_id: i64, text: &str, markup: Option<Value>) -> anyhow::Result<i64> {
        let mut last_id = 0;
        let chunks = chunk_text(text, CHUNK);
        let n = chunks.len();
        for (i, part) in chunks.into_iter().enumerate() {
            let mut body = json!({ "chat_id": chat_id, "text": part, "disable_web_page_preview": true });
            // Botões só na última parte
            if i + 1 == n {
                if let Some(m) = &markup {
                    body["reply_markup"] = m.clone();
                }
            }
            let msg: Message = self.call_with_retries("sendMessage", body, Duration::from_secs(15), 3).await?;
            last_id = msg.message_id;
        }
        Ok(last_id)
    }

    /// Substitui o que o Telegram tinha registrado pra esse token (de qualquer
    /// app anterior) pela lista real deste bot. Falha só gera um aviso no log —
    /// o bot segue funcionando por mensagem normal mesmo sem o menu "/" certo.
    async fn register_commands(&self) {
        let commands: Vec<Value> =
            BOT_COMMANDS.iter().map(|(cmd, desc)| json!({ "command": cmd, "description": desc })).collect();
        if let Err(e) =
            self.call::<bool>("setMyCommands", json!({ "commands": commands }), Duration::from_secs(15)).await
        {
            tracing::warn!(error = %e, "telegram: falha ao registrar os comandos (menu \"/\" pode mostrar lixo de outro app)");
        }
    }

    fn targets(&self, to: &Recipient) -> Vec<i64> {
        match to {
            Recipient::Broadcast => self.allowed.clone(),
            Recipient::User(id) => id.parse().into_iter().collect(),
        }
    }

    fn main_keyboard() -> Value {
        json!({
            "keyboard": [
                [{"text": "📊 /status"}, {"text": "📋 /queue"}],
                [{"text": "🔀 /prs"}, {"text": "⏳ /pending"}],
                [{"text": "📬 /issue"}, {"text": "🏷 /assign"}],
                [{"text": "🛠 /skills"}, {"text": "❓ /help"}]
            ],
            "resize_keyboard": true,
            "input_field_placeholder": "Descreva uma tarefa ou use um comando..."
        })
    }

    /// "📊 /status" → "/status"
    fn normalize(text: &str) -> String {
        let t = text.trim();
        match t.find('/') {
            Some(i) if t[..i].chars().all(|c| !c.is_alphanumeric()) => t[i..].trim().to_string(),
            _ => t.to_string(),
        }
    }

    async fn handle_update(&self, u: Update, tx: &mpsc::Sender<Incoming>) -> anyhow::Result<()> {
        if let Some(q) = u.callback_query {
            let _ = self
                .call::<bool>("answerCallbackQuery", json!({ "callback_query_id": q.id }), Duration::from_secs(10))
                .await;
            if !self.is_allowed(q.from.id) {
                return Ok(());
            }
            if let (Some(data), Some(msg)) = (q.data, q.message) {
                tx.send(Incoming::new(Source::Telegram, msg.chat.id.to_string(), data)).await?;
            }
            return Ok(());
        }

        let Some(m) = u.message else { return Ok(()) };
        let Some(text) = m.text.as_deref().map(Self::normalize).filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let from = m.from.as_ref().map(|f| f.id).unwrap_or(m.chat.id);

        if !self.is_allowed(from) {
            tracing::warn!(user = from, "telegram: acesso negado");
            self.send_to(
                m.chat.id,
                &format!("⛔ Acesso não autorizado.\nSeu ID é {from}. Adicione-o em TELEGRAM_ALLOWED para liberar."),
                None,
            )
            .await?;
            return Ok(());
        }

        if text == "/start" {
            self.send_to(
                m.chat.id,
                "🤖 OREIA-SECA online.\nDescreva uma tarefa em linguagem natural ou toque num comando.",
                Some(Self::main_keyboard()),
            )
            .await?;
            return Ok(());
        }

        tx.send(Incoming::new(Source::Telegram, m.chat.id.to_string(), text)).await?;
        Ok(())
    }
}

#[async_trait]
impl Channel for Telegram {
    fn name(&self) -> &'static str {
        "telegram"
    }

    fn source(&self) -> Source {
        Source::Telegram
    }

    async fn send(&self, to: &Recipient, text: &str) -> anyhow::Result<()> {
        for chat in self.targets(to) {
            if let Err(e) = self.send_to(chat, text, None).await {
                tracing::warn!(chat, error = %e, "telegram: falha ao enviar");
            }
        }
        Ok(())
    }

    async fn typing(&self, to: &Recipient) {
        for chat in self.targets(to) {
            let _ = self
                .call::<bool>("sendChatAction", json!({ "chat_id": chat, "action": "typing" }), Duration::from_secs(5))
                .await;
        }
    }

    async fn ask_approval(&self, task_id: &str, text: &str) -> anyhow::Result<()> {
        let buttons = json!({
            "inline_keyboard": [[
                {"text": "✅ Aprovar", "callback_data": format!("/approve {task_id}")},
                {"text": "❌ Rejeitar", "callback_data": format!("/reject {task_id}")}
            ]]
        });
        let mut sent = Vec::new();
        for chat in &self.allowed {
            match self.send_to(*chat, text, Some(buttons.clone())).await {
                Ok(id) => sent.push((*chat, id)),
                Err(e) => tracing::warn!(chat, error = %e, "telegram: falha ao pedir aprovação"),
            }
        }
        self.approval_msgs.lock().expect("lock").insert(task_id.to_string(), sent);
        Ok(())
    }

    async fn ask_pick(&self, to: &Recipient, text: &str, options: &[(String, String)]) -> anyhow::Result<()> {
        // callback_data do Telegram limita a 64 bytes; comandos de uso real (repo,
        // agente, número de issue/PR) ficam bem abaixo disso, então corto por segurança
        // em vez de falhar a chamada inteira por causa de um nome fora do comum.
        let rows: Vec<Value> = options
            .iter()
            .map(|(label, cmd)| json!([{ "text": label, "callback_data": cmd.chars().take(64).collect::<String>() }]))
            .collect();
        let markup = json!({ "inline_keyboard": rows });
        for chat in self.targets(to) {
            if let Err(e) = self.send_to(chat, text, Some(markup.clone())).await {
                tracing::warn!(chat, error = %e, "telegram: falha ao perguntar");
            }
        }
        Ok(())
    }

    async fn approval_resolved(&self, task_id: &str, note: &str) {
        let msgs = self.approval_msgs.lock().expect("lock").remove(task_id).unwrap_or_default();
        for (chat, id) in msgs {
            let _ = self
                .call::<Value>(
                    "editMessageReplyMarkup",
                    json!({ "chat_id": chat, "message_id": id, "reply_markup": {"inline_keyboard": []} }),
                    Duration::from_secs(10),
                )
                .await;
            let _ = self.send_to(chat, note, None).await;
        }
    }

    async fn run(self: Arc<Self>, tx: mpsc::Sender<Incoming>) -> anyhow::Result<()> {
        if self.allowed.is_empty() {
            tracing::warn!("telegram: TELEGRAM_ALLOWED vazio, nenhum usuário poderá usar o bot");
        }
        self.register_commands().await;
        let mut offset = 0i64;
        let mut backoff = Duration::from_secs(1);
        tracing::info!("telegram: escutando (long polling)");

        loop {
            let body = json!({ "offset": offset, "timeout": 30, "allowed_updates": ["message", "callback_query"] });
            match self.call::<Vec<Update>>("getUpdates", body, Duration::from_secs(40)).await {
                Ok(updates) => {
                    backoff = Duration::from_secs(1);
                    for u in updates {
                        offset = offset.max(u.update_id + 1);
                        if let Err(e) = self.handle_update(u, &tx).await {
                            if tx.is_closed() {
                                return Ok(());
                            }
                            tracing::warn!(error = %e, "telegram: erro ao tratar update");
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("Conflict") {
                        tracing::warn!("telegram: conflito de polling, outra instância rodando?");
                    } else {
                        tracing::warn!(error = %msg, "telegram: falha no polling");
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
            if tx.is_closed() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_allowed_ids() {
        assert_eq!(Telegram::parse_allowed("1, 22 ,x,333"), vec![1, 22, 333]);
        assert!(Telegram::parse_allowed("").is_empty());
    }

    #[test]
    fn normalize_keyboard_buttons() {
        assert_eq!(Telegram::normalize("📊 /status"), "/status");
        assert_eq!(Telegram::normalize("/approve task-1"), "/approve task-1");
        assert_eq!(Telegram::normalize("crie a api /v1/users"), "crie a api /v1/users");
    }

    #[test]
    fn targets() {
        let t = Telegram::with_api("http://x", vec![1, 2]);
        assert_eq!(t.targets(&Recipient::Broadcast), vec![1, 2]);
        assert_eq!(t.targets(&Recipient::user("5")), vec![5]);
        assert!(t.targets(&Recipient::user("abc")).is_empty());
        assert!(t.is_allowed(2) && !t.is_allowed(3));
    }

    /// Serve `/ping`: as primeiras `fail_times` chamadas demoram mais que o
    /// timeout do cliente (simulando uma falha de transporte); depois respondem rápido.
    async fn flaky_server(fail_times: usize) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let app = axum::Router::new().route(
            "/ping",
            axum::routing::post(move || {
                let calls = calls2.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    if n < fail_times {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    axum::Json(json!({"ok": true, "result": true}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), calls)
    }

    #[tokio::test]
    async fn call_with_retries_recovers_after_a_transient_failure() {
        let (api, calls) = flaky_server(2).await;
        let t = Telegram::with_api(&api, vec![1]);
        let ok: bool = t
            .call_with_retries("ping", json!({}), Duration::from_millis(100), 3)
            .await
            .expect("deveria se recuperar dentro das 3 tentativas");
        assert!(ok);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn call_with_retries_gives_up_after_exhausting_attempts() {
        let (api, calls) = flaky_server(usize::MAX).await;
        let t = Telegram::with_api(&api, vec![1]);
        // O servidor nunca responde a tempo: as 3 tentativas falham, e é isso que importa aqui
        // (a mensagem exata de erro do reqwest varia por plataforma, não é o que testamos).
        assert!(t.call_with_retries::<bool>("ping", json!({}), Duration::from_millis(100), 3).await.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3, "deveria tentar exatamente 3 vezes");
    }

    /// Regressão: sem isso, o menu "/" do Telegram mostra o que ficou registrado
    /// por qualquer OUTRO bot/app que já usou esse token — "lixo" de fora deste projeto.
    #[tokio::test]
    async fn register_commands_sends_this_projects_command_list() {
        let calls: Arc<Mutex<Vec<Value>>> = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().route(
            "/setMyCommands",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let c = c.clone();
                async move {
                    c.lock().unwrap().push(body);
                    axum::Json(json!({"ok": true, "result": true}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let t = Telegram::with_api(&format!("http://{addr}"), vec![1]);

        t.register_commands().await;

        let sent = calls.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "deveria registrar uma vez só");
        let commands = sent[0]["commands"].as_array().unwrap();
        assert_eq!(commands.len(), BOT_COMMANDS.len());
        assert!(commands.iter().any(|c| c["command"] == "issue"));
        assert!(commands.iter().any(|c| c["command"] == "assign"));
        // Nenhum comando com "/" ou maiúscula — o Telegram rejeita o `setMyCommands`
        // inteiro se um só comando da lista for inválido.
        assert!(BOT_COMMANDS.iter().all(|(cmd, _)| cmd.chars().all(|c| c.is_ascii_lowercase() || c == '_')));
    }
}
