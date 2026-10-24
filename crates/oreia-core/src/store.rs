//! Armazenamento de tarefas: um arquivo JSON por tarefa em `<dir>/<id>.json`.

use std::path::{Path, PathBuf};

use crate::task::{Task, TaskStatus};

#[derive(Clone)]
pub struct TaskStore {
    dir: PathBuf,
}

impl TaskStore {
    pub async fn open(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "id de tarefa inválido: {id}"
        );
        Ok(self.dir.join(format!("{id}.json")))
    }

    pub async fn save(&self, task: &Task) -> anyhow::Result<()> {
        let path = self.path_for(&task.id)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(task)?).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    pub async fn load(&self, id: &str) -> anyhow::Result<Option<Task>> {
        let path = self.path_for(id)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Todas as tarefas, das mais novas para as mais antigas.
    pub async fn list(&self) -> anyhow::Result<Vec<Task>> {
        let mut out = Vec::new();
        let mut rd = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match tokio::fs::read(&p).await.map(|b| serde_json::from_slice::<Task>(&b)) {
                Ok(Ok(t)) => out.push(t),
                Ok(Err(e)) => tracing::warn!(path = %p.display(), error = %e, "tarefa ilegível"),
                Err(e) => tracing::warn!(path = %p.display(), error = %e, "falha ao ler tarefa"),
            }
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        Ok(out)
    }

    pub async fn list_by_user(&self, user_id: &str) -> anyhow::Result<Vec<Task>> {
        Ok(self.list().await?.into_iter().filter(|t| t.user_id == user_id).collect())
    }

    pub async fn list_by_status(&self, status: TaskStatus) -> anyhow::Result<Vec<Task>> {
        Ok(self.list().await?.into_iter().filter(|t| t.status == status).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Source;

    #[tokio::test]
    async fn save_load_list() {
        let dir = std::env::temp_dir().join(format!("oreia-store-{}", uuid::Uuid::new_v4()));
        let store = TaskStore::open(&dir).await.unwrap();

        let mut t = Task::new(Source::Telegram, "u1", "objetivo");
        store.save(&t).await.unwrap();
        t.set_status(TaskStatus::Running);
        store.save(&t).await.unwrap();

        let loaded = store.load(&t.id).await.unwrap().unwrap();
        assert_eq!(loaded.status, TaskStatus::Running);
        assert_eq!(store.list_by_user("u1").await.unwrap().len(), 1);
        assert_eq!(store.list_by_status(TaskStatus::Running).await.unwrap().len(), 1);
        assert!(store.load("task-nada").await.unwrap().is_none());
        assert!(store.load("../etc/passwd").await.is_err());

        tokio::fs::remove_dir_all(dir).await.ok();
    }
}
