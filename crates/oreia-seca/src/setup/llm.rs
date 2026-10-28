//! Coleta a configuração de um LLM externo — usado tanto pelo `oreia-seca
//! setup llm` standalone quanto pelo wizard unificado (`oreia-seca setup`).
//! Não grava nada em disco por si só; veja `env_updates`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Resultado da coleta: URL + modelos + chave, prontos para `env_updates`.
#[derive(Debug, Clone)]
pub struct Collected {
    pub url: String,
    pub coder_model: String,
    pub chat_model: String,
    pub api_key: Option<String>,
}

impl Collected {
    /// `LLM_CODER_URL`/`LLM_CODER_MODEL` (planner, dev-*, executor) e
    /// `LLM_CHAT_URL`/`LLM_CHAT_MODEL` (direct), prontos para `env_file::write_env_updates`.
    pub fn env_updates(&self) -> Vec<(String, String)> {
        let mut v = vec![
            ("LLM_CODER_URL".to_string(), self.url.clone()),
            ("LLM_CODER_MODEL".to_string(), self.coder_model.clone()),
            ("LLM_CHAT_URL".to_string(), self.url.clone()),
            ("LLM_CHAT_MODEL".to_string(), self.chat_model.clone()),
        ];
        if let Some(k) = &self.api_key {
            v.push(("LLM_API_KEY".to_string(), k.clone()));
        }
        v
    }
}

/// Coleta URL, modelos e chave. Cada argumento `Some` pula a pergunta
/// correspondente; com `url` e `coder_model` juntos, não pergunta nada (modo
/// não interativo, usado em scripts e pelo arquivo de respostas).
pub async fn collect(
    url_arg: Option<&str>,
    coder_model_arg: Option<&str>,
    chat_model_arg: Option<&str>,
    api_key_arg: Option<&str>,
) -> anyhow::Result<Collected> {
    let non_interactive = url_arg.is_some() && coder_model_arg.is_some();

    let url = match url_arg {
        Some(u) => u.to_string(),
        None => ask_url()?,
    };
    let api_key = match api_key_arg {
        Some(k) if !k.is_empty() => Some(k.to_string()),
        Some(_) => None,
        None if non_interactive || looks_local(&url) => None,
        None => ask_api_key()?,
    };

    let available = fetch_and_report(&url, api_key.as_deref()).await;

    let coder_model = match coder_model_arg {
        Some(m) => m.to_string(),
        None => ask_model("código (planner, dev-*, executor)", &available)?,
    };
    let chat_model = match chat_model_arg {
        Some(m) if !m.is_empty() => m.to_string(),
        _ if non_interactive => coder_model.clone(),
        _ => ask_model_with_default("conversa direta (direct)", &available, &coder_model)?,
    };

    Ok(Collected { url, coder_model, chat_model, api_key })
}

async fn fetch_and_report(url: &str, api_key: Option<&str>) -> Vec<String> {
    match fetch_models(url, api_key).await {
        Ok(m) if !m.is_empty() => {
            println!("  servidor respondeu com {} modelo(s): {}", m.len(), m.join(", "));
            m
        }
        Ok(_) => {
            println!("  ⚠ {url}/models respondeu sem nenhum modelo listado.");
            Vec::new()
        }
        Err(e) => {
            println!("  ⚠ não consegui listar os modelos em {url}/models: {e:#}");
            Vec::new()
        }
    }
}

/// Aproximação: trata como "rede local" hosts que claramente não exigem
/// expor uma chave pela internet. Impreciso de propósito — é só para decidir
/// se pula a pergunta da chave, nunca para decidir segurança. Reaproveitado
/// por `crate::check` para não repetir chamadas de geração (custo) contra
/// modelos pagos remotos.
pub(crate) fn looks_local(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
    host == "localhost"
        || host == "host.docker.internal"
        || host.starts_with("127.")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.starts_with("172.")
}

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

async fn fetch_models(base_url: &str, api_key: Option<&str>) -> anyhow::Result<Vec<String>> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build()?;
    let mut req = client.get(format!("{}/models", base_url.trim_end_matches('/')));
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await?.error_for_status()?;
    let parsed: ModelsResponse = resp.json().await?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

fn ask(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn ask_url() -> anyhow::Result<String> {
    println!("URL base do servidor (compatível com /v1/chat/completions), por exemplo:");
    println!("  Ollama em outra máquina:  http://<ip>:11434/v1");
    println!("  LM Studio na rede local:  http://<ip>:1234/v1");
    println!("  OpenRouter:               https://openrouter.ai/api/v1");
    let url = ask("URL: ")?;
    anyhow::ensure!(!url.is_empty(), "a URL não pode ser vazia");
    Ok(url)
}

fn ask_api_key() -> anyhow::Result<Option<String>> {
    let key = ask("Chave da API (Enter se o servidor não exigir): ")?;
    Ok(if key.is_empty() { None } else { Some(key) })
}

fn list_models(label: &str, available: &[String]) {
    if available.is_empty() {
        return;
    }
    println!("Modelos disponíveis para {label}:");
    for (i, m) in available.iter().enumerate() {
        println!("  {}) {m}", i + 1);
    }
}

fn pick(answer: &str, available: &[String]) -> String {
    match answer.parse::<usize>() {
        Ok(n) if n >= 1 && n <= available.len() => available[n - 1].clone(),
        _ => answer.to_string(),
    }
}

fn ask_model(label: &str, available: &[String]) -> anyhow::Result<String> {
    list_models(label, available);
    let answer = ask(&format!("Modelo para {label} (número da lista ou nome): "))?;
    anyhow::ensure!(!answer.is_empty(), "o modelo não pode ser vazio");
    Ok(pick(&answer, available))
}

fn ask_model_with_default(label: &str, available: &[String], default: &str) -> anyhow::Result<String> {
    list_models(label, available);
    let answer = ask(&format!("Modelo para {label} [Enter = {default}]: "))?;
    if answer.is_empty() {
        return Ok(default.to_string());
    }
    Ok(pick(&answer, available))
}

/// Confirma a configuração como `--check`, usando as variáveis que acabamos
/// de gravar (o processo atual não relê o `.env` sozinho).
fn print_check(collected: &Collected, config_path: &Path) {
    for (k, v) in collected.env_updates() {
        std::env::set_var(k, v);
    }

    if !config_path.exists() {
        println!("\n(crie {} a partir de oreia-seca.example.toml para usar essas variáveis)", config_path.display());
        return;
    }
    match crate::config::Config::load(config_path) {
        Ok(cfg) => {
            println!("\n✓ {} válido com as novas variáveis:", config_path.display());
            for (key, spec) in &cfg.models {
                match spec.resolve() {
                    Ok(r) => println!("  modelo {key}: {} → {}", r.model, r.base_url),
                    Err(var) => println!("  modelo {key}: não configurado (falta {var})"),
                }
            }
        }
        Err(e) => println!("\n⚠ {} não carregou: {e:#}", config_path.display()),
    }
}

#[derive(Debug, Default, Clone)]
struct CliArgs {
    url: Option<String>,
    model: Option<String>,
    chat_model: Option<String>,
    api_key: Option<String>,
}

fn parse(args: &[String]) -> anyhow::Result<CliArgs> {
    let mut a = CliArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--url" => a.url = Some(next_value(&mut it, "--url")?),
            "--model" => a.model = Some(next_value(&mut it, "--model")?),
            "--chat-model" => a.chat_model = Some(next_value(&mut it, "--chat-model")?),
            "--api-key" => a.api_key = Some(next_value(&mut it, "--api-key")?),
            other => anyhow::bail!("argumento desconhecido: {other}"),
        }
    }
    Ok(a)
}

fn next_value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> anyhow::Result<String> {
    it.next().cloned().ok_or_else(|| anyhow::anyhow!("{flag} precisa de um valor"))
}

/// `oreia-seca setup llm` standalone: só a parte do LLM, sem Telegram nem Forgejo.
pub async fn run(args: &[String]) -> anyhow::Result<()> {
    let a = parse(args)?;
    println!("🔧 Configurando um LLM externo para o OREIA-SECA.\n");
    let collected =
        collect(a.url.as_deref(), a.model.as_deref(), a.chat_model.as_deref(), a.api_key.as_deref()).await?;
    apply_to_disk(&collected, Path::new(".env"), Path::new(".env.example")).await
}

pub(crate) async fn apply_to_disk(
    collected: &Collected,
    env_path: &Path,
    env_example_path: &Path,
) -> anyhow::Result<()> {
    super::env_file::write_env_updates(env_path, env_example_path, &collected.env_updates())?;
    println!("\n✅ {} atualizado (backup em {}.bak, se já existia).", env_path.display(), env_path.display());
    let config_path =
        crate::config::env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into());
    print_check(collected, &config_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fake_models_server(ids: Vec<&'static str>) -> String {
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(move || {
                let ids = ids.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "data": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    #[test]
    fn looks_local_detects_private_hosts() {
        assert!(looks_local("http://127.0.0.1:11434/v1"));
        assert!(looks_local("http://192.168.1.10:1234/v1"));
        assert!(looks_local("http://localhost:11434/v1"));
        assert!(looks_local("http://host.docker.internal:1234/v1"));
        assert!(!looks_local("https://openrouter.ai/api/v1"));
    }

    #[tokio::test]
    async fn fetch_models_parses_openai_style_response() {
        let url = fake_models_server(vec!["llama3", "qwen2.5-coder"]).await;
        let models = fetch_models(&url, None).await.unwrap();
        assert_eq!(models, vec!["llama3".to_string(), "qwen2.5-coder".to_string()]);
    }

    #[tokio::test]
    async fn non_interactive_collect_defaults_chat_model_to_coder() {
        let url = fake_models_server(vec!["llama3"]).await;
        let collected = collect(Some(&url), Some("llama3"), None, None).await.unwrap();
        assert_eq!(collected.chat_model, "llama3");
        assert_eq!(collected.api_key, None);
    }

    #[tokio::test]
    async fn non_interactive_collect_honors_explicit_chat_model_and_key() {
        let url = fake_models_server(vec!["llama3"]).await;
        let collected = collect(Some(&url), Some("llama3"), Some("mixtral"), Some("s3cr3t")).await.unwrap();
        assert_eq!(collected.chat_model, "mixtral");
        assert_eq!(collected.api_key, Some("s3cr3t".to_string()));
    }

    #[tokio::test]
    async fn apply_to_disk_writes_env_preserves_lines_and_backs_up() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-setup-llm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join(".env");
        let example_path = dir.join(".env.example");
        std::fs::write(&env_path, "TELEGRAM_TOKEN=abc\nOTHER=1\n").unwrap();

        let url = fake_models_server(vec!["llama3"]).await;
        let collected = collect(Some(&url), Some("llama3"), None, None).await.unwrap();
        apply_to_disk(&collected, &env_path, &example_path).await.unwrap();

        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("TELEGRAM_TOKEN=abc"), "{text}");
        assert!(text.contains("OTHER=1"), "{text}");
        assert!(text.contains(&format!("LLM_CODER_URL={url}")), "{text}");
        assert!(text.contains("LLM_CODER_MODEL=llama3"), "{text}");
        assert!(text.contains(&format!("LLM_CHAT_URL={url}")), "{text}");
        assert!(text.contains("LLM_CHAT_MODEL=llama3"), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
