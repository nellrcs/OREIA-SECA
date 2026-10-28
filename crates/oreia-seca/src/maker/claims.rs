//! Posse persistente de um item do Forgejo (issue ou PR), por `dono/repo#n`.
//!
//! Antes disso era só um `Mutex<HashSet<String>>` em memória — bastava pra via
//! automática (uma posse não sobrevive nem precisa sobreviver a um restart: o
//! agente simplesmente recomeça do zero). A via MCP muda isso: uma posse pode
//! durar dias (entre `checkout_issue` e `submit_issue`/merge), então precisa
//! sobreviver a reiniciar o processo. `data/claims.json`, escrita atômica
//! (tmp + rename), mesmo padrão de `oreia_core::store::TaskStore`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Owner {
    /// Posse da via automática: qual agente está com o item.
    Agent { name: String },
    /// Posse tomada por uma ferramenta MCP (`checkout_issue`).
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub owner: Owner,
    pub since: DateTime<Utc>,
    /// Id da `Task` que está rodando (via automática) — usado na reconciliação de boot.
    #[serde(default)]
    pub task_id: Option<String>,
}

pub struct ClaimStore {
    path: PathBuf,
    claims: Mutex<BTreeMap<String, Claim>>,
}

impl ClaimStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let claims = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, claims: Mutex::new(claims) })
    }

    pub fn key(repo_full: &str, issue: u64) -> String {
        format!("{repo_full}#{issue}")
    }

    fn save(&self, map: &BTreeMap<String, Claim>) {
        if let Err(e) = self.save_inner(map) {
            tracing::warn!(error = %e, path = %self.path.display(), "falha ao gravar claims.json");
        }
    }

    fn save_inner(&self, map: &BTreeMap<String, Claim>) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(map)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Toma a posse. `false` se o item já estava com alguém (quem chama decide a mensagem).
    pub fn claim(&self, repo_full: &str, issue: u64, owner: Owner, task_id: Option<String>) -> bool {
        let key = Self::key(repo_full, issue);
        let mut map = self.claims.lock().expect("lock de claims");
        if map.contains_key(&key) {
            return false;
        }
        map.insert(key, Claim { owner, since: Utc::now(), task_id });
        self.save(&map);
        true
    }

    pub fn release(&self, repo_full: &str, issue: u64) {
        let key = Self::key(repo_full, issue);
        let mut map = self.claims.lock().expect("lock de claims");
        if map.remove(&key).is_some() {
            self.save(&map);
        }
    }

    pub fn get(&self, repo_full: &str, issue: u64) -> Option<Claim> {
        self.claims.lock().expect("lock de claims").get(&Self::key(repo_full, issue)).cloned()
    }

    /// Todas as posses atuais (chave `dono/repo#n` → posse) — reconciliação de boot e `/status`.
    pub fn all(&self) -> Vec<(String, Claim)> {
        self.claims.lock().expect("lock de claims").iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn remove_key(&self, key: &str) {
        let mut map = self.claims.lock().expect("lock de claims");
        if map.remove(key).is_some() {
            self.save(&map);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "oreia-claims-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn claim_release_and_reopen_from_disk() {
        let p = path();
        let store = ClaimStore::open(p.clone()).unwrap();
        assert!(store.claim("acme/app", 7, Owner::Agent { name: "dev-back".into() }, Some("task-1".into())));
        assert!(!store.claim("acme/app", 7, Owner::Mcp, None), "já estava com dev-back");
        assert!(store.get("acme/app", 7).is_some());

        // Reabre do disco (simula restart do processo) e confere que a posse sobreviveu.
        let reopened = ClaimStore::open(p.clone()).unwrap();
        let claim = reopened.get("acme/app", 7).unwrap();
        assert!(matches!(claim.owner, Owner::Agent { name } if name == "dev-back"));

        reopened.release("acme/app", 7);
        assert!(reopened.get("acme/app", 7).is_none());
        assert!(ClaimStore::open(p.clone()).unwrap().get("acme/app", 7).is_none());

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn mcp_claim_survives_alongside_agent_claims() {
        let p = path();
        let store = ClaimStore::open(p.clone()).unwrap();
        store.claim("acme/app", 1, Owner::Agent { name: "dev-back".into() }, Some("task-1".into()));
        store.claim("acme/app", 2, Owner::Mcp, None);
        let all = store.all();
        assert_eq!(all.len(), 2);
        std::fs::remove_file(&p).ok();
    }
}
