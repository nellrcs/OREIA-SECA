//! `oreia-seca setup --web`: formulário numa página local, para quem prefere
//! não usar o terminal. Mesma aplicação de respostas do wizard e do arquivo
//! (`apply_answers`) — esta camada só coleta via HTML em vez de `stdin`.
//!
//! Sem autenticação: pensado para `127.0.0.1` (padrão). Se ligar em outro
//! host, qualquer um que alcançar a porta grava o `.env` da sua máquina.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::{apply_answers, Answers, ForgejoAnswers, LlmAnswers, TelegramAnswers};

struct WebState {
    env_path: PathBuf,
    env_example_path: PathBuf,
    bootstrap_script: PathBuf,
    /// Tomado e usado uma vez, no primeiro envio do formulário, para desligar o servidor.
    done: Mutex<Option<oneshot::Sender<()>>>,
}

pub async fn run(host: &str, port: u16) -> anyhow::Result<()> {
    let addr = format!("{host}:{port}");
    let listener =
        TcpListener::bind(&addr).await.map_err(|e| anyhow::anyhow!("não consegui escutar em {addr}: {e}"))?;
    let real_addr = listener.local_addr()?;

    println!("🔧 Abra http://{real_addr}/ no navegador para configurar o OREIA-SECA.");
    if host != "127.0.0.1" && host != "localhost" {
        println!(
            "⚠ Escutando em {host}: qualquer um que alcançar essa porta pode gravar o .env desta máquina. \
             Sem autenticação — use só numa rede confiável, de preferência 127.0.0.1."
        );
    }
    println!("Ctrl+C encerra sem salvar nada.\n");

    serve(listener, PathBuf::from(".env"), PathBuf::from(".env.example"), PathBuf::from("scripts/bootstrap.sh")).await
}

async fn serve(
    listener: TcpListener,
    env_path: PathBuf,
    env_example_path: PathBuf,
    bootstrap_script: PathBuf,
) -> anyhow::Result<()> {
    let (tx, rx) = oneshot::channel();
    let state = Arc::new(WebState { env_path, env_example_path, bootstrap_script, done: Mutex::new(Some(tx)) });

    let app = Router::new().route("/", get(form_page).post(submit)).with_state(state);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = rx.await;
        })
        .await?;
    Ok(())
}

async fn form_page() -> Html<String> {
    Html(page(FORM_BODY))
}

#[derive(Debug, Default, Deserialize)]
struct SubmitForm {
    #[serde(default)]
    telegram_token: String,
    #[serde(default)]
    telegram_allowed: String,
    #[serde(default)]
    llm_url: String,
    #[serde(default)]
    llm_model: String,
    #[serde(default)]
    llm_chat_model: String,
    #[serde(default)]
    llm_api_key: String,
    /// Checkbox HTML: presente e "on" quando marcado, ausente quando não.
    #[serde(default)]
    forgejo_bootstrap: String,
}

impl From<SubmitForm> for Answers {
    fn from(f: SubmitForm) -> Self {
        Answers {
            telegram: TelegramAnswers { token: f.telegram_token, allowed: f.telegram_allowed },
            llm: LlmAnswers {
                url: f.llm_url,
                model: f.llm_model,
                chat_model: f.llm_chat_model,
                api_key: f.llm_api_key,
            },
            forgejo: ForgejoAnswers { bootstrap: f.forgejo_bootstrap == "on" },
        }
    }
}

async fn submit(State(state): State<Arc<WebState>>, Form(form): Form<SubmitForm>) -> Html<String> {
    let answers: Answers = form.into();
    let result = apply_answers(&answers, &state.env_path, &state.env_example_path, &state.bootstrap_script).await;

    // Primeiro envio desliga o servidor (depois de esta resposta terminar de ser entregue).
    if let Some(tx) = state.done.lock().expect("lock do estado web").take() {
        let _ = tx.send(());
    }

    Html(page(&result_body(&result)))
}

fn result_body(result: &anyhow::Result<super::ApplyReport>) -> String {
    match result {
        Ok(report) => {
            let mut items = Vec::new();
            if report.llm_configured {
                items.push("<li>✅ LLM configurado no <code>.env</code>.</li>".to_string());
            }
            if report.telegram_configured {
                items.push("<li>✅ Telegram configurado no <code>.env</code>.</li>".to_string());
            }
            if report.bootstrap_ran {
                items.push(format!(
                    "<li>✅ Bootstrap do Forgejo rodou:<pre>{}</pre></li>",
                    html_escape(report.bootstrap_output.trim())
                ));
            }
            for n in &report.notes {
                items.push(format!("<li>ℹ {}</li>", html_escape(n)));
            }
            format!(
                "<h1>Pronto</h1><ul>{}</ul><p>Pode fechar esta página. Rode <code>make up</code> (ou <code>make check</code> para confirmar antes).</p>",
                items.join("")
            )
        }
        Err(e) => format!(
            "<h1>Algo deu errado</h1><p><code>{}</code></p><p><a href=\"/\">Voltar</a></p>",
            html_escape(&e.to_string())
        ),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const FORM_BODY: &str = r#"
<h1>Configuração inicial do OREIA-SECA</h1>
<p>Preencha o que quiser agora; o que ficar em branco não é alterado. Veja
<code>docs/instalacao.md</code> para o que cada campo faz.</p>
<form method="post">
  <fieldset>
    <legend>Telegram (opcional)</legend>
    <label>Token do bot (@BotFather)<br><input type="text" name="telegram_token" size="50"></label><br><br>
    <label>Seu ID numérico (vírgula se forem vários)<br><input type="text" name="telegram_allowed" size="50"></label>
  </fieldset>
  <fieldset>
    <legend>LLM externo</legend>
    <label>URL base (ex.: http://&lt;ip&gt;:11434/v1)<br><input type="text" name="llm_url" size="50"></label><br><br>
    <label>Modelo para código (planner, dev-*, executor)<br><input type="text" name="llm_model" size="50"></label><br><br>
    <label>Modelo para conversa direta (em branco = o mesmo de cima)<br><input type="text" name="llm_chat_model" size="50"></label><br><br>
    <label>Chave da API (em branco se o servidor não exigir)<br><input type="password" name="llm_api_key" size="50"></label>
  </fieldset>
  <fieldset>
    <legend>Forgejo</legend>
    <label><input type="checkbox" name="forgejo_bootstrap"> Rodar o bootstrap agora (sobe Postgres+Forgejo, cria contas/tokens/repos — precisa de Docker e pode levar alguns minutos)</label>
  </fieldset>
  <br>
  <button type="submit">Aplicar</button>
</form>
"#;

fn page(body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="pt-BR"><head><meta charset="utf-8">
<title>OREIA-SECA — setup</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 640px; margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; }}
  fieldset {{ margin-bottom: 1rem; }}
  input {{ font-size: 1rem; padding: 0.3rem; }}
  button {{ font-size: 1rem; padding: 0.5rem 1.5rem; }}
  pre {{ background: #f0f0f0; padding: 0.5rem; overflow-x: auto; }}
  code {{ background: #f0f0f0; padding: 0 0.2rem; }}
</style>
</head><body>{body}</body></html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn form_submission_writes_env_and_shuts_down_the_server() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-setup-web-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join(".env");
        let example_path = dir.join(".env.example");
        let bootstrap_script = dir.join("missing-bootstrap.sh");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(serve(listener, env_path.clone(), example_path.clone(), bootstrap_script.clone()));

        // GET / mostra o formulário.
        let client = reqwest::Client::new();
        let got = client.get(format!("http://{addr}/")).send().await.unwrap();
        assert!(got.status().is_success());
        assert!(got.text().await.unwrap().contains("llm_model"));

        // POST aplica e desliga o servidor.
        let resp = client
            .post(format!("http://{addr}/"))
            .form(&[
                ("telegram_token", "tok"),
                ("telegram_allowed", "1,2"),
                ("llm_url", "http://x/v1"),
                ("llm_model", "m"),
                ("llm_chat_model", ""),
                ("llm_api_key", ""),
            ])
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert!(body.contains("Pronto"), "{body}");

        tokio::time::timeout(std::time::Duration::from_secs(5), handle).await.unwrap().unwrap().unwrap();

        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("TELEGRAM_TOKEN=tok"), "{text}");
        assert!(text.contains("LLM_CODER_MODEL=m"), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn checkbox_value_maps_to_bootstrap_flag() {
        let on = SubmitForm { forgejo_bootstrap: "on".into(), ..Default::default() };
        let off = SubmitForm { forgejo_bootstrap: String::new(), ..Default::default() };
        assert!(Answers::from(on).forgejo.bootstrap);
        assert!(!Answers::from(off).forgejo.bootstrap);
    }
}
