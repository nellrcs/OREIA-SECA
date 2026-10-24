//! Skills definidas em Markdown.
//!
//! No OREIA-SECA cada skill trazia um bloco de JavaScript. Aqui o bloco é de
//! shell (```sh ou ```bash), e cada parâmetro chega como variável de ambiente
//! `P_<NOME>` (ex.: `city` vira `$P_CITY`). Isso evita interpolar texto do
//! modelo dentro do script, o que abriria injeção de comandos.
//!
//! Formato:
//!
//! ````markdown
//! # skill_weather
//!
//! Consulta a previsão do tempo de uma cidade.
//!
//! ## Parâmetros
//! - `city` (string, obrigatório): nome da cidade
//!
//! ```sh
//! curl -s "wttr.in/$P_CITY?format=3"
//! ```
//! ````

use std::path::Path;

use async_trait::async_trait;

use super::builtin::{run_command, sandboxed_shell};
use super::{Action, ActionContext, ParamSpec, Params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSkill {
    name: String,
    description: String,
    params: Vec<ParamSpec>,
    script: String,
}

impl ShellSkill {
    pub fn script(&self) -> &str {
        &self.script
    }
}

pub fn parse_skill(md: &str) -> Option<ShellSkill> {
    let md = md.replace("\r\n", "\n");
    let mut lines = md.lines().peekable();

    // # nome
    let name = loop {
        let l = lines.next()?;
        if let Some(rest) = l.strip_prefix("# ") {
            let n = rest.trim();
            if !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_') {
                break n.to_string();
            }
            return None;
        }
    };

    let mut description = Vec::new();
    let mut params = Vec::new();
    let mut script = None;
    let mut section = "desc".to_string();

    while let Some(l) = lines.next() {
        if let Some(h) = l.strip_prefix("## ") {
            section = h.trim().to_lowercase();
            continue;
        }
        let fence = l.trim_start();
        if fence.starts_with("```sh") || fence.starts_with("```bash") || fence.starts_with("```shell") {
            let mut body = Vec::new();
            for b in lines.by_ref() {
                if b.trim_start().starts_with("```") {
                    break;
                }
                body.push(b);
            }
            script = Some(body.join("\n"));
            continue;
        }
        match section.as_str() {
            "desc" => description.push(l),
            "parâmetros" | "parametros" | "parameters" => {
                if let Some(p) = parse_param_line(l) {
                    params.push(p);
                }
            }
            _ => {}
        }
    }

    Some(ShellSkill {
        name,
        description: description.join("\n").trim().to_string(),
        params,
        script: script?.trim().to_string(),
    })
}

/// `- \`nome\` (tipo, obrigatório): descrição`
fn parse_param_line(line: &str) -> Option<ParamSpec> {
    let l = line.trim().strip_prefix("- ")?.trim();
    let l = l.strip_prefix('`')?;
    let (name, rest) = l.split_once('`')?;
    let rest = rest.trim();
    let (meta, desc) = match rest.strip_prefix('(') {
        Some(r) => {
            let (meta, after) = r.split_once(')')?;
            (meta.to_lowercase(), after.trim().trim_start_matches(':').trim())
        }
        None => (String::new(), rest.trim_start_matches(':').trim()),
    };
    let required = meta.contains("obrigat") || meta.contains("required");
    Some(ParamSpec::new(name.trim(), desc, required))
}

/// Carrega `dir/*/skill.md` (ou `SKILL.md`) e `dir/*.md`.
pub fn load_dir(dir: &Path) -> Vec<ShellSkill> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            for f in ["skill.md", "SKILL.md"] {
                let c = p.join(f);
                if c.is_file() {
                    files.push(c);
                    break;
                }
            }
        } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
            files.push(p);
        }
    }
    files.sort();
    files
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(&f).ok()?;
            let s = parse_skill(&text);
            if s.is_none() {
                tracing::warn!(file = %f.display(), "skill ignorada: formato inválido");
            }
            s
        })
        .collect()
}

#[async_trait]
impl Action for ShellSkill {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn params(&self) -> Vec<ParamSpec> {
        self.params.clone()
    }
    async fn run(&self, p: &Params, ctx: &ActionContext) -> anyhow::Result<String> {
        let mut cmd = sandboxed_shell(&self.script, ctx);
        for (k, v) in p {
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                cmd.env(format!("P_{}", k.to_ascii_uppercase()), v);
            }
        }
        run_command(cmd, &self.name, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MD: &str = "# skill_echo\n\nRepete um texto.\nEm duas linhas.\n\n## Parâmetros\n- `text` (string, obrigatório): o texto\n- `times` (number, opcional): repetições\n\n## Código\n```sh\necho \"eco: $P_TEXT\"\n```\n";

    #[test]
    fn parses_markdown() {
        let s = parse_skill(MD).unwrap();
        assert_eq!(s.name(), "skill_echo");
        assert_eq!(s.description(), "Repete um texto.\nEm duas linhas.");
        assert_eq!(s.params().len(), 2);
        assert!(s.params()[0].required);
        assert!(!s.params()[1].required);
        assert_eq!(s.script(), "echo \"eco: $P_TEXT\"");
    }

    #[test]
    fn rejects_without_script() {
        assert!(parse_skill("# x\n\nsem código").is_none());
        assert!(parse_skill("sem título").is_none());
    }

    #[tokio::test]
    async fn params_are_env_not_interpolated() {
        let s = parse_skill(MD).unwrap();
        let mut p = Params::new();
        p.insert("text".into(), "oi; rm -rf /tmp/nada".into());
        let ctx = ActionContext {
            task_id: None,
            user_id: "u".into(),
            workdir: std::env::temp_dir(),
            shell_timeout: Duration::from_secs(5),
        };
        let out = s.run(&p, &ctx).await.unwrap();
        assert!(out.contains("eco: oi; rm -rf /tmp/nada"), "{out}");
    }
}
