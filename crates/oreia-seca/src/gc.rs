//! `oreia-seca gc` (`./oreia gc`): apaga clones de tarefas do Forgejo que
//! sobraram em `[paths].workspace` — de antes da limpeza automática existir
//! ([`crate::maker::Maker::cleanup_workdir`], chamada ao fim de cada tarefa),
//! de uma queda no meio de uma tarefa, ou de `[paths].keep_workdir = true`
//! que você desligou depois. Veja docs/problemas.md#disco.
//!
//! Só remove o que dá pra confirmar que é seguro pelo registro da própria
//! tarefa ([`oreia_core::TaskStore`]): tem `work_item` (então já foi
//! empurrado para o Forgejo) e já terminou. Uma tarefa direta (chat, sem
//! issue) nunca é tocada — o workspace dela É o resultado, sem outra cópia.

use std::path::{Path, PathBuf};

use oreia_core::store::TaskStore;

#[derive(Debug, Default)]
pub struct GcReport {
    /// Id da tarefa e quantos bytes o clone ocupava.
    pub removed: Vec<(String, u64)>,
    /// Id da tarefa e por que ficou — não é erro, é prudência.
    pub kept: Vec<(String, String)>,
}

impl GcReport {
    pub fn freed_bytes(&self) -> u64 {
        self.removed.iter().map(|(_, b)| b).sum()
    }
}

/// Varre `workspace` por diretórios `task-*` e apaga os que pertencem a uma
/// tarefa do Forgejo já terminada. Diretório sem `task-` no nome, ou sem
/// registro correspondente no `store`, fica — pode ser de outra coisa.
pub async fn sweep(workspace: &Path, store: &TaskStore) -> anyhow::Result<GcReport> {
    let mut report = GcReport::default();
    let mut rd = match tokio::fs::read_dir(workspace).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        let Ok(ft) = entry.file_type().await else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        if !id.starts_with("task-") {
            continue;
        }
        match store.load(&id).await {
            Ok(Some(task)) if task.work_item.is_none() => {
                report.kept.push((id, "tarefa direta (sem issue) — é o próprio resultado, não apago".into()));
            }
            Ok(Some(task)) if !task.status.is_finished() => {
                report.kept.push((id, format!("ainda está \"{}\" — pode estar em uso", task.status.as_str())));
            }
            Ok(Some(_)) => match remove_dir(&path).await {
                Ok(bytes) => report.removed.push((id, bytes)),
                Err(e) => report.kept.push((id, format!("falha ao apagar: {e}"))),
            },
            Ok(None) => report.kept.push((id, "sem registro da tarefa (data/tasks) — não arrisco apagar".into())),
            Err(e) => report.kept.push((id, format!("não consegui ler o registro: {e}"))),
        }
    }
    report.removed.sort();
    report.kept.sort();
    Ok(report)
}

async fn remove_dir(path: &Path) -> std::io::Result<u64> {
    let bytes = dir_size(path).await.unwrap_or(0);
    tokio::fs::remove_dir_all(path).await?;
    Ok(bytes)
}

async fn dir_size(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(entry.path());
            } else {
                total += entry.metadata().await?.len();
            }
        }
    }
    Ok(total)
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `oreia-seca gc`: carrega a configuração do processo e imprime o relatório.
pub async fn run_cli() -> anyhow::Result<()> {
    let toml = crate::config::env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into());
    let cfg = crate::config::Config::load(&toml)?;
    let store = TaskStore::open(cfg.tasks_dir()).await?;
    let report = sweep(&cfg.paths.workspace, &store).await?;

    if report.removed.is_empty() {
        println!("Nada para limpar em {} — já estava em dia.", cfg.paths.workspace.display());
    } else {
        println!("Limpos {} clone(s), {} liberado(s):", report.removed.len(), human_bytes(report.freed_bytes()));
        for (id, bytes) in &report.removed {
            println!("  ✓ {id} ({})", human_bytes(*bytes));
        }
    }
    if !report.kept.is_empty() {
        println!("\nMantidos (por prudência):");
        for (id, reason) in &report.kept {
            println!("  · {id}: {reason}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oreia_core::{Source, Task, TaskStatus, WorkItem, WorkMode};

    fn work_item() -> WorkItem {
        WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 1,
            agent: "dev-back".into(),
            title: "x".into(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        }
    }

    async fn write_dir(path: &Path, bytes: &[u8]) {
        tokio::fs::create_dir_all(path).await.unwrap();
        tokio::fs::write(path.join("file.txt"), bytes).await.unwrap();
    }

    #[tokio::test]
    async fn removes_finished_forgejo_tasks_keeps_everything_else() {
        let root = std::env::temp_dir().join(format!("oreia-gc-{}", uuid::Uuid::new_v4().simple()));
        let workspace = root.join("workspace");
        let store = TaskStore::open(root.join("data/tasks")).await.unwrap();

        // 1) tarefa do Forgejo, terminada — deve ser apagada.
        let mut done = Task::new(Source::Forgejo, "u", "g");
        done.work_item = Some(work_item());
        done.set_status(TaskStatus::Done);
        store.save(&done).await.unwrap();
        write_dir(&workspace.join(&done.id), b"0123456789").await;

        // 2) tarefa direta (sem issue), terminada — NUNCA apagada (é o próprio resultado).
        let mut direct = Task::new(Source::Telegram, "u", "g");
        direct.set_status(TaskStatus::Done);
        store.save(&direct).await.unwrap();
        write_dir(&workspace.join(&direct.id), b"0123456789").await;

        // 3) tarefa do Forgejo ainda rodando — não apagada.
        let mut running = Task::new(Source::Forgejo, "u", "g");
        running.work_item = Some(work_item());
        running.set_status(TaskStatus::Running);
        store.save(&running).await.unwrap();
        write_dir(&workspace.join(&running.id), b"x").await;

        // 4) diretório sem registro nenhum — não apagado.
        write_dir(&workspace.join("task-orfao"), b"x").await;

        // 5) diretório que nem é de tarefa — ignorado silenciosamente.
        write_dir(&workspace.join("not-a-task"), b"x").await;

        let report = sweep(&workspace, &store).await.unwrap();

        assert_eq!(report.removed, vec![(done.id.clone(), 10)]);
        assert!(!workspace.join(&done.id).exists());
        assert!(workspace.join(&direct.id).exists());
        assert!(workspace.join(&running.id).exists());
        assert!(workspace.join("task-orfao").exists());
        assert!(workspace.join("not-a-task").exists());
        assert_eq!(report.kept.len(), 3, "{:?}", report.kept);

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn missing_workspace_dir_is_not_an_error() {
        let root = std::env::temp_dir().join(format!("oreia-gc-missing-{}", uuid::Uuid::new_v4().simple()));
        let store = TaskStore::open(root.join("data/tasks")).await.unwrap();
        let report = sweep(&root.join("no-such-workspace"), &store).await.unwrap();
        assert!(report.removed.is_empty());
        assert!(report.kept.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn human_bytes_picks_the_right_unit() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
