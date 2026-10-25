//! Edição de `.env` preservando as outras linhas. Usado por todos os modos de
//! setup (wizard de terminal, formulário web, arquivo de respostas) para que
//! os três caiam no mesmo código de gravação.

use std::path::{Path, PathBuf};

/// Lê o valor de `key` no `.env`, se o arquivo e a linha existirem e não estiver vazia.
/// Usado por `oreia-seca status`/`up` para saber que porta checar, sem precisar
/// carregar o `.env` no processo (este binário não faz isso sozinho).
pub fn read_env_value(env_path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(env_path).ok()?;
    let prefix = format!("{key}=");
    text.lines().find_map(|l| l.strip_prefix(prefix.as_str())).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn backup_path(env_path: &Path) -> PathBuf {
    let mut s = env_path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

fn set_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    match lines.iter().position(|l| l.starts_with(&prefix)) {
        Some(i) => lines[i] = format!("{key}={value}"),
        None => lines.push(format!("{key}={value}")),
    }
}

/// Aplica `updates` (`chave`, `valor`) no `.env`, preservando as outras linhas.
/// Faz backup em `<env_path>.bak` se o arquivo já existia; sem ele, parte do
/// `.env.example` (como `scripts/bootstrap.sh` já faz para o resto do `.env`).
/// Sem nenhuma atualização, não toca no arquivo.
pub fn write_env_updates(env_path: &Path, env_example_path: &Path, updates: &[(String, String)]) -> anyhow::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let existing_text = if env_path.exists() {
        let text = std::fs::read_to_string(env_path)?;
        std::fs::copy(env_path, backup_path(env_path))?;
        text
    } else if env_example_path.exists() {
        std::fs::read_to_string(env_example_path)?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = existing_text.lines().map(String::from).collect();
    for (key, value) in updates {
        set_line(&mut lines, key, value);
    }
    std::fs::write(env_path, format!("{}\n", lines.join("\n")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("oreia-seca-envfile-{label}-{}", std::process::id()))
    }

    #[test]
    fn read_env_value_finds_or_skips_lines() {
        let d = dir("read");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(&env_path, "FORGEJO_HTTP_PORT=3010\nEMPTY=\n").unwrap();
        assert_eq!(read_env_value(&env_path, "FORGEJO_HTTP_PORT"), Some("3010".to_string()));
        assert_eq!(read_env_value(&env_path, "EMPTY"), None);
        assert_eq!(read_env_value(&env_path, "MISSING"), None);
        assert_eq!(read_env_value(&d.join("nope.env"), "X"), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn no_updates_leaves_file_untouched() {
        let d = dir("noop");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        write_env_updates(&env_path, &d.join(".env.example"), &[]).unwrap();
        assert!(!env_path.exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn preserves_other_lines_and_backs_up() {
        let d = dir("preserve");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(&env_path, "TELEGRAM_TOKEN=abc\nOTHER=1\n").unwrap();

        write_env_updates(
            &env_path,
            &d.join(".env.example"),
            &[("LLM_CODER_URL".into(), "http://x/v1".into()), ("OTHER".into(), "2".into())],
        )
        .unwrap();

        let backup = backup_path(&env_path);
        assert!(backup.exists());
        assert!(std::fs::read_to_string(&backup).unwrap().contains("OTHER=1"));

        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("TELEGRAM_TOKEN=abc"), "{text}");
        assert!(text.contains("LLM_CODER_URL=http://x/v1"), "{text}");
        assert!(text.contains("OTHER=2"), "{text}"); // atualizado no lugar, não duplicado
        assert_eq!(text.matches("OTHER=").count(), 1, "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn creates_from_example_when_env_is_missing() {
        let d = dir("fromexample");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        let example_path = d.join(".env.example");
        std::fs::write(&example_path, "TELEGRAM_TOKEN=\n").unwrap();

        write_env_updates(&env_path, &example_path, &[("LLM_CODER_MODEL".into(), "llama3".into())]).unwrap();

        assert!(!backup_path(&env_path).exists(), "não devia haver backup sem .env prévio");
        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("TELEGRAM_TOKEN="), "{text}");
        assert!(text.contains("LLM_CODER_MODEL=llama3"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }
}
