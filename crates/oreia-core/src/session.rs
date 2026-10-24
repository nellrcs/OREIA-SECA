//! Histórico de conversa por sessão (`fonte:usuário`), persistido em JSON.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::message::ChatMessage;

pub struct SessionStore {
    path: Option<PathBuf>,
    inner: Mutex<HashMap<String, Vec<ChatMessage>>>,
    max_messages: usize,
}

impl SessionStore {
    /// Sessões só em memória (útil em testes).
    pub fn in_memory() -> Self {
        Self { path: None, inner: Mutex::new(HashMap::new()), max_messages: 200 }
    }

    /// Carrega do disco se o arquivo existir.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data = match std::fs::read_to_string(&path) {
            Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s)?,
            _ => HashMap::new(),
        };
        Ok(Self { path: Some(path), inner: Mutex::new(data), max_messages: 200 })
    }

    pub fn get(&self, key: &str) -> Vec<ChatMessage> {
        self.inner.lock().expect("lock de sessões").get(key).cloned().unwrap_or_default()
    }

    pub fn push(&self, key: &str, msg: ChatMessage) {
        let mut g = self.inner.lock().expect("lock de sessões");
        let v = g.entry(key.to_string()).or_default();
        v.push(msg);
        let excess = v.len().saturating_sub(self.max_messages);
        if excess > 0 {
            v.drain(..excess);
        }
    }

    pub fn replace(&self, key: &str, msgs: Vec<ChatMessage>) {
        self.inner.lock().expect("lock de sessões").insert(key.to_string(), msgs);
    }

    pub fn clear(&self, key: &str) {
        self.inner.lock().expect("lock de sessões").remove(key);
    }

    /// Grava no disco de forma atômica (arquivo temporário + rename).
    pub fn flush(&self) -> anyhow::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = {
            let g = self.inner.lock().expect("lock de sessões");
            serde_json::to_string_pretty(&*g)?
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

/// Estimativa grosseira de tokens (4 caracteres ≈ 1 token), como no original.
pub fn estimate_tokens(msgs: &[ChatMessage]) -> usize {
    msgs.iter().map(|m| m.content.chars().count().div_ceil(4) + 4).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_get_clear() {
        let s = SessionStore::in_memory();
        s.push("telegram:1", ChatMessage::user("oi"));
        s.push("telegram:1", ChatMessage::assistant("olá"));
        assert_eq!(s.get("telegram:1").len(), 2);
        s.clear("telegram:1");
        assert!(s.get("telegram:1").is_empty());
    }

    #[test]
    fn persists_to_disk() {
        let dir = std::env::temp_dir().join(format!("oreia-sess-{}", uuid::Uuid::new_v4()));
        let file = dir.join("sessions.json");
        {
            let s = SessionStore::open(&file).unwrap();
            s.push("k", ChatMessage::user("x"));
            s.flush().unwrap();
        }
        let s = SessionStore::open(&file).unwrap();
        assert_eq!(s.get("k")[0].content, "x");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn estimate() {
        assert_eq!(estimate_tokens(&[ChatMessage::user("abcdefgh")]), 2 + 4);
    }
}
