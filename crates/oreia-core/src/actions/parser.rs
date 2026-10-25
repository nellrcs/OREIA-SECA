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
static SPECIAL_CLOSING_RE: OnceLock<Regex> = OnceLock::new();
static SPECIAL_OPENING_NAMED_RE: OnceLock<Regex> = OnceLock::new();
static SPECIAL_OPENING_BARE_RE: OnceLock<Regex> = OnceLock::new();

const Q_OPEN: &str = "\"'\u{201C}\u{2018}";
const Q_CLOSE: &str = "\"'\u{201D}\u{2019}";

/// Regressão real (visto em produção com deepseek no papel `reviewer`): o modelo vaza
/// tokens internos do PRÓPRIO formato de tool-calling no lugar do que pedimos — fechamento
/// (`<param name="command">ls -a</｜DSML｜ parameter>` em vez de `</param>`, ou
/// `</action>\n</｜DSML｜ calls>` em vez de só `</action>`) E abertura (visto depois, com
/// DOIS `<action>` na mesma resposta: `<｜DSML｜ parameter name="command" string="true">ls -la`
/// em vez de `<param name="command">ls -la`). Qualquer um dos dois faz o parser não achar o
/// parâmetro (mesmo o modelo tendo escrito o valor certo) e a ação falhar com "parâmetro
/// obrigatório ausente". Normaliza pelo que a palavra vazada claramente tenta dizer, antes
/// de qualquer parsing.
pub fn normalize_model_markup(text: &str) -> String {
    if !text.contains('\u{FF5C}') {
        return text.to_string();
    }
    let closing = re(&SPECIAL_CLOSING_RE, "</\u{FF5C}[^\u{FF5C}>]*\u{FF5C}\\s*(\\w*)>");
    let s = closing.replace_all(text, |caps: &regex::Captures| {
        let word = caps[1].to_lowercase();
        if word.contains("param") {
            "</param>".to_string()
        } else if word.contains("invoke") || word.contains("action") {
            // "invoke" fecha UMA chamada de ferramenta — exatamente o que `</action>`
            // delimita aqui. "calls" (sem "invoke"/"action") fecha o LOTE inteiro de
            // chamadas, sem equivalente nosso, então cai no `else` e é só removido.
            "</action>".to_string()
        } else {
            String::new()
        }
    });

    // Abertura com `name="x"` embutido (e possivelmente outros atributos vazados, ex.
    // `string="true"`, descartados): reconstrói só o que o parser de `<param>` espera.
    let opening_named = re(
        &SPECIAL_OPENING_NAMED_RE,
        "<\u{FF5C}[^\u{FF5C}>]*\u{FF5C}\\s+(\\w+)[^>]*\\bname\\s*=\\s*\"([^\"]*)\"[^>]*>",
    );
    let s = opening_named.replace_all(&s, |caps: &regex::Captures| {
        if caps[1].to_lowercase().contains("param") {
            format!("<param name=\"{}\">", &caps[2])
        } else {
            String::new()
        }
    });

    // Qualquer outro token de abertura sobrando (ex. `<｜DSML｜ invoke>`, sem `name=`): ruído
    // de fronteira entre chamadas, sem equivalente nosso — remove.
    let opening_bare = re(&SPECIAL_OPENING_BARE_RE, "<\u{FF5C}[^\u{FF5C}>]*\u{FF5C}[^>]*>");
    opening_bare.replace_all(&s, "").into_owned()
}

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

    /// Regressão real (produção, papel `reviewer`, deepseek): o modelo vazou
    /// `</｜DSML｜ parameter>` no lugar de `</param>`. Sem normalizar, o parâmetro "command"
    /// ficava invisível pro parser e a ação falhava com "parâmetro obrigatório ausente",
    /// mesmo o modelo tendo escrito o comando certo. Texto exato capturado do log de verdade.
    #[test]
    fn normalizes_deepseek_leaked_param_closing_tag() {
        let raw = "<action name=\"shell\">\n  <param name=\"command\">git diff origin/main...HEAD</\u{FF5C}DSML\u{FF5C} parameter>\n</action>";
        let a = parse_actions(&normalize_model_markup(raw));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].params["command"], "git diff origin/main...HEAD");
    }

    /// Mesma classe, mas fechando o `<action>` ("calls", não "parameter") — também visto
    /// em produção, logo após um bloco já corretamente fechado por `</action>`.
    #[test]
    fn normalizes_deepseek_leaked_action_closing_tag() {
        let raw =
            "<action name=\"shell\">\n<param name=\"command\">ls -la</param>\n</action>\n</\u{FF5C}DSML\u{FF5C} calls>";
        let normalized = normalize_model_markup(raw);
        assert!(!normalized.contains('\u{FF5C}'), "{normalized}");
        let a = parse_actions(&normalized);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].params["command"], "ls -la");
    }

    /// Regressão real (produção, mesma classe, variante mais grave): o modelo vazou a
    /// tag de ABERTURA também — `<｜DSML｜ parameter name="command" string="true">` em vez
    /// de `<param name="command">` — misturada com um `<param>` normal no mesmo texto.
    /// Texto exato capturado do log de verdade (duas chamadas de `shell`).
    #[test]
    fn normalizes_deepseek_leaked_param_opening_tag() {
        let raw = "<action name=\"shell\">\n<param name=\"command\">git status</\u{FF5C}DSML\u{FF5C} parameter>\n</\u{FF5C}DSML\u{FF5C} invoke>\n<action name=\"shell\">\n<\u{FF5C}DSML\u{FF5C} parameter name=\"command\" string=\"true\">ls -la</\u{FF5C}DSML\u{FF5C} parameter>\n</\u{FF5C}DSML\u{FF5C} invoke>\n</\u{FF5C}DSML\u{FF5C} calls>";
        let normalized = normalize_model_markup(raw);
        assert!(!normalized.contains('\u{FF5C}'), "{normalized}");
        let a = parse_actions(&normalized);
        assert_eq!(a.len(), 2, "{normalized}");
        assert_eq!(a[0].params["command"], "git status");
        assert_eq!(a[1].params["command"], "ls -la");
    }

    #[test]
    fn normalize_is_a_no_op_without_the_special_token() {
        let raw = "<action name=\"shell\"><param name=\"command\">ls</param></action>";
        assert_eq!(normalize_model_markup(raw), raw);
    }

    #[test]
    fn multiple_and_narrative() {
        let t = "Primeiro.\n<action name=\"a\"></action>\n\n\n\nDepois.<action name=\"b\"></action>";
        assert_eq!(parse_actions(t).len(), 2);
        assert_eq!(parse_narrative(t), "Primeiro.\n\nDepois.");
    }
}
