//! Wizard de terminal do `oreia-seca setup` (sem argumentos): pergunta
//! Telegram, LLM e Forgejo em sequência e aplica tudo no final.

use std::io::Write;
use std::path::Path;

use super::{apply_answers, llm, print_report, Answers, ForgejoAnswers, LlmAnswers, TelegramAnswers};

pub async fn run(env_path: &Path, env_example_path: &Path, bootstrap_script: &Path) -> anyhow::Result<()> {
    println!("🔧 Configuração inicial do OREIA-SECA\n");
    println!("Cobre Telegram, o LLM externo e, se quiser, o bootstrap do Forgejo.");
    println!("Enter sem digitar nada pula uma pergunta opcional.\n");

    println!("── Telegram (opcional) ──");
    let token = ask_line("Token do bot (@BotFather, Enter para pular): ")?;
    let allowed =
        if token.is_empty() { String::new() } else { ask_line("Seu ID numérico (vírgula se forem vários): ")? };

    println!("\n── LLM externo ──");
    let collected = llm::collect(None, None, None, None).await?;

    println!("\n── Forgejo ──");
    let bootstrap =
        ask_yes_no("Rodar o bootstrap do Forgejo agora (sobe Postgres+Forgejo, cria contas/tokens/repos)? [s/N]: ")?;

    let answers = Answers {
        telegram: TelegramAnswers { token, allowed },
        llm: LlmAnswers {
            url: collected.url,
            model: collected.coder_model,
            chat_model: collected.chat_model,
            api_key: collected.api_key.unwrap_or_default(),
        },
        forgejo: ForgejoAnswers { bootstrap },
    };

    let report = apply_answers(&answers, env_path, env_example_path, bootstrap_script).await?;
    print_report(&report);
    Ok(())
}

fn ask_line(prompt: &str) -> anyhow::Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn ask_yes_no(prompt: &str) -> anyhow::Result<bool> {
    let a = ask_line(prompt)?;
    Ok(matches!(a.to_lowercase().as_str(), "s" | "sim" | "y" | "yes"))
}
