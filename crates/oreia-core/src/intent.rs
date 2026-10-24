//! Classificador de intenção por pontuação de sinais (portado do OREIA-SECA).
//! Decide se uma mensagem vira tarefa planejada ou resposta direta.
//!
//! Diferença em relação ao original: as palavras de escopo são comparadas
//! por palavra inteira, para "com" não casar dentro de "comando".

const ACTION_VERBS: &[&str] = &[
    "criar",
    "cria",
    "crie",
    "desenvolver",
    "desenvolva",
    "implementar",
    "implementa",
    "implemente",
    "construir",
    "constrói",
    "construa",
    "fazer",
    "faz",
    "faça",
    "gerar",
    "gera",
    "gere",
    "escrever",
    "escreve",
    "escreva",
    "montar",
    "monta",
    "monte",
    "configurar",
    "configura",
    "configure",
    "estruturar",
    "adicionar",
    "adicione",
    "corrigir",
    "corrija",
    "refatorar",
    "refatore",
];

const TECH_NOUNS: &[&str] = &[
    "landing",
    "dashboard",
    "crud",
    "api",
    "rest",
    "servidor",
    "backend",
    "frontend",
    "projeto",
    "sistema",
    "aplicação",
    "app",
    "site",
    "página",
    "script",
    "bot",
    "cli",
    "módulo",
    "biblioteca",
    "componente",
    "serviço",
    "endpoint",
    "tela",
    "feature",
    "funcionalidade",
    "teste",
    "testes",
];

const SCOPE_WORDS: &[&str] =
    &["completo", "completa", "inteiro", "inteira", "funcional", "com", "incluindo", "além", "também", "mais", "tudo"];

pub const THRESHOLD: u32 = 3;

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Pontuação de complexidade da mensagem.
pub fn score(text: &str) -> u32 {
    let lower = text.to_lowercase();
    let ws = words(text);
    let mut s = 0;

    if ACTION_VERBS.iter().any(|v| ws.iter().any(|w| w == v)) {
        s += 3;
    }
    if TECH_NOUNS.iter().any(|n| lower.contains(n)) {
        s += 2;
    }
    s += SCOPE_WORDS.iter().filter(|m| ws.iter().any(|w| w == *m)).count() as u32;

    let len = text.chars().count();
    if len > 60 {
        s += 1;
    }
    if len > 120 {
        s += 1;
    }
    let sentences = text.split(['.', '!', '?']).filter(|p| p.trim().chars().count() > 3).count();
    if sentences >= 2 {
        s += 1;
    }
    s
}

/// `true` quando a mensagem deve virar uma tarefa com plano.
pub fn is_complex(text: &str) -> bool {
    score(text) >= THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_messages_are_direct() {
        assert!(!is_complex("oi, tudo bem?"));
        assert!(!is_complex("que horas são"));
        assert!(!is_complex("qual o comando para listar containers"));
    }

    #[test]
    fn build_requests_are_tasks() {
        assert!(is_complex("crie uma api rest completa com autenticação"));
        assert!(is_complex("implemente a tela de login"));
        assert!(is_complex("refatore o módulo de pagamentos"));
    }

    #[test]
    fn scope_words_match_whole_words_only() {
        // "comando" não deve contar como "com"
        assert_eq!(score("comando"), 0);
    }
}
