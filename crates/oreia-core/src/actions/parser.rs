//! Parser tolerante de `<action>` (portado do OREIA-SECA).
//!
//! Modelos locais pequenos erram o formato com frequência, então aceitamos:
//! 1. atributos na tag de abertura: `<action name="shell" command="ls">`
//! 2. `<param name="x">valor</param>`
//! 3. tags diretas: `<path>a.txt</path>`
//! 4. linhas `chave: valor` (só preenchem o que ainda falta)
//! 5. um objeto JSON no corpo (sobrescreve tudo)

use std::sync::OnceLock;

use regex::Regex;

use super::Params;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAction {
    pub name: String,
    pub params: Params,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("regex inválida"))
}

static ACTION_RE: OnceLock<Regex> = OnceLock::new();
static ATTR_RE: OnceLock<Regex> = OnceLock::new();
static PARAM_RE: OnceLock<Regex> = OnceLock::new();
static DIRECT_RE: OnceLock<Regex> = OnceLock::new();
static KV_RE: OnceLock<Regex> = OnceLock::new();
static STRIP_RE: OnceLock<Regex> = OnceLock::new();
static BLANKS_RE: OnceLock<Regex> = OnceLock::new();

const Q_OPEN: &str = "\"'\u{201C}\u{2018}";
const Q_CLOSE: &str = "\"'\u{201D}\u{2019}";

pub fn parse_actions(text: &str) -> Vec<ParsedAction> {
    let action_re = re(
        &ACTION_RE,
        &format!(r#"(?is)<action\s+([^>]*?)name\s*=\s*[{Q_OPEN}]?(\w+)[{Q_CLOSE}]?([^>]*)>(.*?)</action>"#),
    );
    let attr_re = re(&ATTR_RE, &format!(r#"(\w+)\s*=\s*(?:[{Q_OPEN}]([^{Q_CLOSE}>]+)[{Q_CLOSE}]|([^\s>]+))"#));
    let param_re = re(&PARAM_RE, &format!(r#"(?is)<param\s+name\s*=\s*[{Q_OPEN}]?(\w+)[{Q_CLOSE}]?\s*>(.*?)</param>"#));
    let direct_re = re(&DIRECT_RE, r"(?is)<(\w+)\s*>(.*?)</(\w+)>");
    let kv_re = re(&KV_RE, r"^\s*(\w+)\s*[:=]\s*(.+)$");

    let mut out = Vec::new();
    for cap in action_re.captures_iter(text) {
        let name = cap[2].to_string();
        let attrs = format!("{} {}", &cap[1], &cap[3]);
        let body = &cap[4];
        let mut params = Params::new();

        // 1. atributos da tag
        for a in attr_re.captures_iter(&attrs) {
            let key = a[1].to_lowercase();
            if key == "name" {
                continue;
            }
            if let Some(v) = a.get(2).or_else(|| a.get(3)) {
                params.insert(key, v.as_str().trim().to_string());
            }
        }

        // 2. <param name="x">
        for p in param_re.captures_iter(body) {
            params.insert(p[1].to_string(), p[2].trim().to_string());
        }

        // 3. tags diretas (sem backreference: compara abertura e fechamento)
        for d in direct_re.captures_iter(body) {
            let open = d[1].to_lowercase();
            if open == d[3].to_lowercase() && open != "param" {
                params.insert(open, d[2].trim().to_string());
            }
        }

        // 4. linhas chave: valor (só se ainda não há nenhum parâmetro vindo das tags)
        let has_tags = !params.is_empty();
        if !has_tags {
            for line in body.lines() {
                if let Some(kv) = kv_re.captures(line) {
                    let key = kv[1].trim().to_lowercase();
                    let val = kv[2]
                        .trim()
                        .trim_start_matches(|c| Q_OPEN.contains(c))
                        .trim_end_matches(|c| Q_CLOSE.contains(c))
                        .to_string();
                    if key != "action" && key != "param" {
                        params.entry(key).or_insert(val);
                    }
                }
            }
        }

        // 5. JSON no corpo
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(body.trim()) {
            for (k, v) in map {
                let v = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                params.insert(k, v);
            }
        }

        out.push(ParsedAction { name, params });
    }
    out
}

/// Texto do modelo sem as tags de ação.
pub fn parse_narrative(text: &str) -> String {
    let strip = re(&STRIP_RE, r"(?is)<action.*?</action>");
    let blanks = re(&BLANKS_RE, r"\n{3,}");
    let s = strip.replace_all(text, "");
    blanks.replace_all(&s, "\n\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_tags() {
        let a = parse_actions(r#"<action name="shell"><param name="command">ls -la</param></action>"#);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "shell");
        assert_eq!(a[0].params["command"], "ls -la");
    }

    #[test]
    fn attributes_and_curly_quotes() {
        let a = parse_actions("<action name=“file_read” path=“src/a.rs”></action>");
        assert_eq!(a[0].name, "file_read");
        assert_eq!(a[0].params["path"], "src/a.rs");
    }

    #[test]
    fn attribute_before_name() {
        let a = parse_actions(r#"<action path="x.txt" name="file_read"></action>"#);
        assert_eq!(a[0].name, "file_read");
        assert_eq!(a[0].params["path"], "x.txt");
    }

    #[test]
    fn direct_tags() {
        let a = parse_actions("<action name=\"file_write\"><path>a.txt</path><content>oi</content></action>");
        assert_eq!(a[0].params["path"], "a.txt");
        assert_eq!(a[0].params["content"], "oi");
    }

    #[test]
    fn key_value_lines() {
        let a = parse_actions("<action name=\"shell\">\ncommand: \"cargo test\"\n</action>");
        assert_eq!(a[0].params["command"], "cargo test");
    }

    #[test]
    fn json_body() {
        let a = parse_actions(r#"<action name="x">{"a": "1", "b": 2}</action>"#);
        assert_eq!(a[0].params["a"], "1");
        assert_eq!(a[0].params["b"], "2");
    }

    #[test]
    fn file_content_with_colons_is_not_polluted() {
        let a = parse_actions(
            "<action name=\"file_write\"><param name=\"path\">c.yml</param><param name=\"content\">image: x\nports: 1</param></action>",
        );
        assert_eq!(a[0].params.len(), 2);
        assert_eq!(a[0].params["content"], "image: x\nports: 1");
    }

    #[test]
    fn multiple_and_narrative() {
        let t = "Primeiro.\n<action name=\"a\"></action>\n\n\n\nDepois.<action name=\"b\"></action>";
        assert_eq!(parse_actions(t).len(), 2);
        assert_eq!(parse_narrative(t), "Primeiro.\n\nDepois.");
    }
}
