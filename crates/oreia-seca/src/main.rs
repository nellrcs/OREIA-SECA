//! OREIA-SECA — orquestrador de agentes de IA.
//!
//! Uso:
//!   oreia-seca [--config oreia-seca.toml]   inicia o orquestrador
//!   oreia-seca --check [--config …]         valida a configuração e mostra o resumo
//!   oreia-seca setup                        wizard de terminal (Telegram, LLM, Forgejo)
//!   oreia-seca setup --answers ARQUIVO.toml  não interativo, a partir de um arquivo (ou flags)
//!   oreia-seca setup llm [--url U --model M] só a parte do LLM
//!   oreia-seca status|up|down                atalhos sem precisar de make/bash (chamam docker compose)
//!   oreia-seca mcp [--config …]              servidor MCP (stdio) — exige [mcp] no config
//!
//! Veja docs/instalacao.md e docs/mcp-plano/.

mod check;
mod config;
mod executor;
mod gitops;
mod http;
mod maker;
mod ops;
mod planner;
mod prompts;
mod setup;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oreia_channels::{Channel, Telegram, Terminal};
use oreia_llm::ModelRouter;
use tokio::sync::mpsc;

use crate::config::{env, Config};
use crate::maker::{ForgejoCtx, Maker};

struct Args {
    config: PathBuf,
    check: bool,
    /// `--check --live`: além de validar a forma, roda as checagens de rede/processo
    /// (ver `crate::check`) contra o que `--since` (ou, sem ele, a config inteira) aponta.
    live: bool,
    since: Option<PathBuf>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut config = env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into());
    let mut check = false;
    let mut live = false;
    let mut since = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" | "-c" => {
                config = it.next().ok_or_else(|| anyhow::anyhow!("--config precisa de um caminho"))?.into()
            }
            "--check" => check = true,
            "--live" => live = true,
            "--since" => {
                since = Some(it.next().ok_or_else(|| anyhow::anyhow!("--since precisa de um caminho"))?.into())
            }
            "--version" | "-V" => {
                println!("oreia-seca {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("uso: oreia-seca [--config oreia-seca.toml] [--check [--live [--since ARQUIVO.toml]]]");
                println!("     oreia-seca setup [--answers ARQUIVO.toml] [--llm-url U --llm-model M ...]");
                println!("     oreia-seca setup llm [--url U --model M] [--api-key K]");
                println!("     oreia-seca status | up | down");
                println!("     oreia-seca mcp [--config oreia-seca.toml]");
                std::process::exit(0);
            }
            other => anyhow::bail!("argumento desconhecido: {other}"),
        }
    }
    Ok(Args { config, check, live, since })
}

/// Caminho de `--config`/`-c` (ou `OREIA_SECA_CONFIG`), igual a `parse_args` — só que
/// sem exigir os outros argumentos (`mcp` não tem `--check`/`--live`).
fn config_path_from_args() -> PathBuf {
    let mut config = env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into());
    let mut it = std::env::args().skip(2); // pula o binário e o "mcp"
    while let Some(a) = it.next() {
        if a == "--config" || a == "-c" {
            if let Some(v) = it.next() {
                config = v.into();
            }
        }
    }
    config
}

/// Servidor MCP (stdio). A STDOUT é o protocolo MCP — por isso o tracing deste
/// subcomando vai inteiro pra stderr, e PRECISA ser configurado antes de qualquer
/// outra coisa (inclusive antes do `tracing_subscriber::fmt()...init()` genérico lá
/// embaixo, que escreve em stdout e corromperia a conexão).
async fn run_mcp() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let config_path = config_path_from_args();
    let cfg = Config::load(&config_path)?;
    let mcp_cfg = cfg
        .mcp
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("defina [mcp] em {} para usar \"oreia-seca mcp\"", config_path.display()))?;
    anyhow::ensure!(mcp_cfg.enabled, "[mcp].enabled = false em {} — a feature está desligada", config_path.display());

    let base_url = format!("http://127.0.0.1:{}", cfg.server.port);
    let api_key = env(&cfg.server.api_key_env);
    if api_key.is_none() {
        tracing::warn!(
            var = %cfg.server.api_key_env,
            "sem token da API REST: o servidor MCP vai chamar /api/mcp/* sem autenticação"
        );
    }

    // Transporte HTTP streamable, opcional e além do stdio (nunca no lugar dele) —
    // só sobe com [mcp].http = true E um MCP_TOKEN configurado; sem o token, recusa
    // já aqui em vez de subir uma porta sem autenticação nenhuma.
    if let Some(mcp_token) = http_transport_token(mcp_cfg.http, env(&mcp_cfg.token_env), &mcp_cfg.token_env)? {
        let (http_base_url, http_api_key, bind) = (base_url.clone(), api_key.clone(), mcp_cfg.bind.clone());
        tokio::spawn(async move {
            if let Err(e) = oreia_mcp::serve_http(http_base_url, http_api_key, bind, mcp_token).await {
                tracing::error!(error = %e, "transporte HTTP do MCP caiu");
            }
        });
    }

    oreia_mcp::serve_stdio(base_url, api_key).await
}

/// `[mcp].http` desligado → `Ok(None)` (não sobe nada). Ligado sem o token definido →
/// `Err` (recusa antes de subir uma porta sem autenticação). Ligado com token →
/// `Ok(Some(token))`. Separada de `run_mcp` só pra dar pra testar sem um `[mcp]` de
/// verdade nem rodar o binário inteiro.
fn http_transport_token(
    http_enabled: bool,
    token_env_value: Option<String>,
    token_env_name: &str,
) -> anyhow::Result<Option<String>> {
    if !http_enabled {
        return Ok(None);
    }
    token_env_value
        .ok_or_else(|| {
            anyhow::anyhow!(
                "[mcp].http = true mas {token_env_name} não está definido — defina um token antes de expor o transporte HTTP"
            )
        })
        .map(Some)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return run_mcp().await;
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("setup") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return setup::run(&rest).await;
        }
        Some("status") | Some("up") | Some("down") => {
            let rest: Vec<String> = std::env::args().skip(1).collect();
            return ops::run(&rest).await;
        }
        _ => {}
    }

    let args = parse_args()?;
    let cfg = Arc::new(Config::load(&args.config)?);

    let router = Arc::new(ModelRouter::from_config(
        &cfg.models,
        &cfg.roles,
        &cfg.llm.fallback,
        Duration::from_secs(cfg.llm.cooldown_secs),
        (cfg.llm.timeout_secs > 0).then(|| Duration::from_secs(cfg.llm.timeout_secs)),
    )?);
    let forgejo = ForgejoCtx::build(&cfg)?;

    if args.check {
        println!("✓ {} válido", args.config.display());
        for (key, spec) in &cfg.models {
            match spec.resolve() {
                Ok(r) => println!("  modelo {key:<12} → {} ({})", r.model, r.base_url),
                Err(var) => println!("  modelo {key:<12} → não configurado (falta a variável {var})"),
            }
        }
        for r in router.describe() {
            println!("  papel {:<10} → {}", r.role, r.model);
        }
        if let Some(f) = &forgejo {
            println!("  forgejo: {} · agentes ativos: {:?}", f.cfg.url, f.agent_names());
        }

        if args.live {
            let targets = match &args.since {
                Some(old_path) => {
                    let old_text = std::fs::read_to_string(old_path)
                        .map_err(|e| anyhow::anyhow!("não consegui ler {}: {e}", old_path.display()))?;
                    let old_cfg: Config =
                        toml::from_str(&old_text).map_err(|e| anyhow::anyhow!("{}: {e}", old_path.display()))?;
                    old_cfg.validate()?;
                    check::diff(&old_cfg, &cfg)
                }
                None => check::all_targets(&cfg),
            };
            if targets.is_empty() {
                println!("\n(nada para checar)");
                return Ok(());
            }
            println!("\nChecando {} alvo(s) ao vivo (pode levar alguns segundos)...", targets.len());
            let results = check::run(&cfg, &targets).await;
            for r in &results {
                println!("  {} {} — {}", r.status.icon(), r.target, r.message);
            }
            let log_path = cfg.paths.data.join("checks.jsonl");
            if let Err(e) = check::log::append(&log_path, &results).await {
                tracing::warn!(error = %e, path = %log_path.display(), "falha ao gravar o histórico de checagens");
            }
        }
        return Ok(());
    }

    // Canais humanos
    let mut channels: Vec<Arc<dyn Channel>> = Vec::new();
    if cfg.telegram.enabled {
        match env(&cfg.telegram.token_env) {
            Some(token) => {
                let allowed = Telegram::parse_allowed(&env(&cfg.telegram.allowed_env).unwrap_or_default());
                channels.push(Arc::new(Telegram::new(&token, allowed)));
            }
            None => tracing::warn!(var = %cfg.telegram.token_env, "Telegram desativado: token ausente"),
        }
    }
    if cfg.terminal.enabled && std::io::stdin().is_terminal() {
        channels.push(Arc::new(Terminal::new()));
    }

    let webhook_secret = forgejo.as_ref().and_then(|f| f.webhook_secret.clone());
    let maker = Maker::new(cfg.clone(), router.clone(), channels.clone(), forgejo).await?;

    // Saúde dos modelos em segundo plano (não trava o boot se o Ollama demorar)
    {
        let router = router.clone();
        tokio::spawn(async move {
            router.check_health().await;
            for r in router.describe() {
                tracing::info!(role = %r.role, model = %r.model, ready = r.ready, "papel");
            }
        });
    }

    let (tx, mut rx) = mpsc::channel(128);
    for ch in &channels {
        let (ch, tx) = (ch.clone(), tx.clone());
        tokio::spawn(async move {
            let name = ch.name();
            if let Err(e) = ch.run(tx).await {
                tracing::error!(channel = name, error = %e, "canal encerrou com erro");
            }
        });
    }

    // API HTTP
    let api_key = env(&cfg.server.api_key_env);
    if api_key.is_none() {
        tracing::warn!(var = %cfg.server.api_key_env, "API REST sem autenticação; defina um token");
    }
    let state = Arc::new(http::HttpState { maker: maker.clone(), tx: tx.clone(), api_key, webhook_secret });
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "API HTTP escutando");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, http::router(state)).await {
            tracing::error!(error = %e, "servidor HTTP caiu");
        }
    });

    maker.recover_interrupted().await;
    drop(tx);

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(m) => { tokio::spawn(maker.clone().handle(m)); }
                None => break,
            },
            _ = shutdown_signal() => {
                tracing::info!("sinal de desligamento recebido");
                break;
            }
        }
    }

    maker.shutdown().await;
    tracing::info!("até logo");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_transport_token_disabled_is_a_no_op() {
        assert!(http_transport_token(false, None, "MCP_TOKEN").unwrap().is_none());
        assert!(http_transport_token(false, Some("x".into()), "MCP_TOKEN").unwrap().is_none());
    }

    #[test]
    fn http_transport_token_enabled_without_token_refuses() {
        let err = http_transport_token(true, None, "MCP_TOKEN").unwrap_err();
        assert!(err.to_string().contains("MCP_TOKEN"), "{err}");
    }

    #[test]
    fn http_transport_token_enabled_with_token_is_some() {
        assert_eq!(http_transport_token(true, Some("tok".into()), "MCP_TOKEN").unwrap(), Some("tok".to_string()));
    }
}
