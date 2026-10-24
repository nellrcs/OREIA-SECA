//! OREIA-SECA — orquestrador de agentes de IA.
//!
//! Uso:
//!   oreia-seca [--config oreia-seca.toml]   inicia o orquestrador
//!   oreia-seca --check [--config …]         valida a configuração e mostra o resumo
//!   oreia-seca setup                        wizard de terminal (Telegram, LLM, Forgejo)
//!   oreia-seca setup --web                  o mesmo wizard, numa página local
//!   oreia-seca setup --answers ARQUIVO.toml  não interativo, a partir de um arquivo (ou flags)
//!   oreia-seca setup llm [--url U --model M] só a parte do LLM
//!
//! Veja docs/instalacao.md.

mod config;
mod executor;
mod gitops;
mod http;
mod maker;
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
}

fn parse_args() -> anyhow::Result<Args> {
    let mut config = env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into());
    let mut check = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" | "-c" => {
                config = it.next().ok_or_else(|| anyhow::anyhow!("--config precisa de um caminho"))?.into()
            }
            "--check" => check = true,
            "--version" | "-V" => {
                println!("oreia-seca {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("uso: oreia-seca [--config oreia-seca.toml] [--check]");
                println!("     oreia-seca setup [--web] [--answers ARQUIVO.toml] [--llm-url U --llm-model M ...]");
                println!("     oreia-seca setup llm [--url U --model M] [--api-key K]");
                std::process::exit(0);
            }
            other => anyhow::bail!("argumento desconhecido: {other}"),
        }
    }
    Ok(Args { config, check })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,oreia=info".into()),
        )
        .with_target(false)
        .init();

    if std::env::args().nth(1).as_deref() == Some("setup") {
        let rest: Vec<String> = std::env::args().skip(2).collect();
        return setup::run(&rest).await;
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
