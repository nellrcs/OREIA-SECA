//! `oreia-seca status|up|down`: wrappers finos sobre `docker compose`, para
//! quem não tem (ou não quer usar) `make`. Só precisam de Docker e de `cargo`
//! para compilar este binário — nada de bash nem de make — então funcionam
//! em qualquer SO onde o Rust e o Docker já estejam instalados.
//!
//! `up` confere antes se as portas configuradas (`FORGEJO_HTTP_PORT`,
//! `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT` no `.env`) já estão ocupadas por
//! outra coisa, e avisa — em vez de deixar o Docker falhar com um erro cru,
//! como aconteceu num teste real deste projeto.

use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use crate::setup::env_file::read_env_value;

pub async fn run(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("status") => status().await,
        Some("up") => up().await,
        Some("down") => compose_ok(&["down"]).await,
        other => anyhow::bail!("comando desconhecido: {other:?} (use status, up ou down)"),
    }
}

async fn compose(args: &[&str]) -> anyhow::Result<ExitStatus> {
    tokio::process::Command::new("docker").arg("compose").args(args).status().await.map_err(|e| {
        anyhow::anyhow!("não consegui chamar \"docker compose\": {e} (o Docker está instalado e no PATH?)")
    })
}

async fn compose_ok(args: &[&str]) -> anyhow::Result<()> {
    let status = compose(args).await?;
    anyhow::ensure!(status.success(), "docker compose {} terminou com código {:?}", args.join(" "), status.code());
    Ok(())
}

async fn service_running(service: &str) -> bool {
    let out = tokio::process::Command::new("docker")
        .args(["compose", "ps", "-q", "--status", "running", service])
        .output()
        .await;
    matches!(out, Ok(o) if !o.stdout.is_empty())
}

async fn port_taken(port: u16) -> bool {
    tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(("127.0.0.1", port)))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

fn parse_port_or_default(value: Option<String>, default: u16) -> u16 {
    value.and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Lê a porta do `.env` na raiz do projeto, ou usa o padrão.
fn env_port(key: &str, default: u16) -> u16 {
    parse_port_or_default(read_env_value(Path::new(".env"), key), default)
}

async fn health_check(label: &str, url: &str) {
    match reqwest::Client::new().get(url).timeout(Duration::from_secs(3)).send().await {
        Ok(r) if r.status().is_success() => println!("✅ {label} respondendo em {url}"),
        Ok(r) => println!("⚠ {label} respondeu {} em {url}", r.status()),
        Err(_) => println!("⚪ {label} não respondeu em {url} (pode não estar rodando, ou ainda subindo)"),
    }
}

async fn status() -> anyhow::Result<()> {
    println!("🔎 Containers (docker compose ps):\n");
    compose_ok(&["ps"]).await?;
    println!();
    health_check("orquestrador", &format!("http://localhost:{}/health", env_port("OREIA_SECA_PORT", 3120))).await;
    health_check("Forgejo", &format!("http://localhost:{}/api/healthz", env_port("FORGEJO_HTTP_PORT", 3000))).await;
    Ok(())
}

async fn up() -> anyhow::Result<()> {
    if !service_running("forgejo").await && !service_running("oreia-seca").await {
        for (label, var, default) in [
            ("HTTP do Forgejo", "FORGEJO_HTTP_PORT", 3000),
            ("SSH do Forgejo", "FORGEJO_SSH_PORT", 2222),
            ("API do orquestrador", "OREIA_SECA_PORT", 3120),
        ] {
            let port = env_port(var, default);
            if port_taken(port).await {
                println!(
                    "⚠ a porta {port} ({label}) já parece estar em uso por outro processo. \
                     Se não for o próprio OREIA-SECA ainda subindo, mude {var} no .env antes de continuar."
                );
            }
        }
    }
    compose_ok(&["up", "-d", "--build"]).await?;
    println!(
        "\n✅ Subiu. \"oreia-seca status\" mostra o estado; \"docker compose logs -f oreia-seca\" acompanha os logs."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_port_or_default_falls_back_on_missing_or_invalid() {
        assert_eq!(parse_port_or_default(None, 3000), 3000);
        assert_eq!(parse_port_or_default(Some("abc".into()), 3000), 3000);
        assert_eq!(parse_port_or_default(Some("3010".into()), 3000), 3010);
    }

    #[tokio::test]
    async fn port_taken_detects_a_bound_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_taken(port).await);
    }
}
