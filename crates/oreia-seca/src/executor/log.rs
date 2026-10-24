//! Registro das tentativas de escalada em `data/escalation.jsonl` (uma linha
//! por tentativa) e agregação para `/stats`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationLogEntry {
    pub task_id: String,
    pub role: String,
    /// 1-based.
    pub phase: usize,
    /// 1-based.
    pub step: usize,
    pub executor: String,
    pub model: String,
    pub passed: bool,
    #[serde(default)]
    pub reason: Option<String>,
    pub duration_ms: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub at: chrono::DateTime<chrono::Utc>,
}

pub async fn append(path: &Path, entries: &[EscalationLogEntry]) -> anyhow::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut buf = String::new();
    for e in entries {
        buf.push_str(&serde_json::to_string(e)?);
        buf.push('\n');
    }
    let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
    f.write_all(buf.as_bytes()).await?;
    Ok(())
}

pub async fn load(path: &Path) -> anyhow::Result<Vec<EscalationLogEntry>> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RoleStats {
    pub role: String,
    /// Quantas fases foram resolvidas (em qualquer degrau).
    pub resolved_phases: usize,
    /// Degrau (1-based) → quantas fases resolvidas nele.
    pub by_step: BTreeMap<usize, usize>,
    pub total_cost_usd: f64,
    pub avg_cost_usd_per_task: f64,
    /// Degrau (1-based) → duração média em ms.
    pub avg_step_duration_ms: BTreeMap<usize, u64>,
}

/// Agrega por papel: % de fases resolvidas em cada degrau, custo total/médio
/// por tarefa e tempo médio por degrau.
pub fn aggregate(entries: &[EscalationLogEntry], role_filter: Option<&str>) -> Vec<RoleStats> {
    let mut by_role: BTreeMap<String, Vec<&EscalationLogEntry>> = BTreeMap::new();
    for e in entries {
        if role_filter.is_some_and(|r| !r.eq_ignore_ascii_case(&e.role)) {
            continue;
        }
        by_role.entry(e.role.clone()).or_default().push(e);
    }

    by_role
        .into_iter()
        .map(|(role, es)| {
            let mut cost_by_task: BTreeMap<&str, f64> = BTreeMap::new();
            let mut by_step: BTreeMap<usize, usize> = BTreeMap::new();
            let mut dur_sum: BTreeMap<usize, (u64, u64)> = BTreeMap::new();
            let mut resolved_phases = 0usize;

            for e in &es {
                *cost_by_task.entry(e.task_id.as_str()).or_insert(0.0) += e.cost_usd;
                let d = dur_sum.entry(e.step).or_insert((0, 0));
                d.0 += e.duration_ms;
                d.1 += 1;
                if e.passed {
                    *by_step.entry(e.step).or_insert(0) += 1;
                    resolved_phases += 1;
                }
            }

            let total_cost_usd: f64 = cost_by_task.values().sum();
            let avg_cost_usd_per_task =
                if cost_by_task.is_empty() { 0.0 } else { total_cost_usd / cost_by_task.len() as f64 };
            let avg_step_duration_ms =
                dur_sum.into_iter().map(|(step, (sum, n))| (step, sum.checked_div(n).unwrap_or(0))).collect();

            RoleStats { role, resolved_phases, by_step, total_cost_usd, avg_cost_usd_per_task, avg_step_duration_ms }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn entry(task: &str, role: &str, step: usize, passed: bool, cost: f64, ms: u64) -> EscalationLogEntry {
        EscalationLogEntry {
            task_id: task.into(),
            role: role.into(),
            phase: 1,
            step,
            executor: "builtin".into(),
            model: "m".into(),
            passed,
            reason: None,
            duration_ms: ms,
            prompt_tokens: 10,
            completion_tokens: 10,
            cost_usd: cost,
            at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn appends_and_loads_jsonl() {
        let path = std::env::temp_dir().join(format!("oreia-seca-esc-{}.jsonl", std::process::id()));
        tokio::fs::remove_file(&path).await.ok();
        append(&path, &[entry("t1", "dev-back", 1, false, 0.1, 100)]).await.unwrap();
        append(&path, &[entry("t1", "dev-back", 2, true, 0.4, 200)]).await.unwrap();
        let loaded = load(&path).await.unwrap();
        assert_eq!(loaded.len(), 2);
        tokio::fs::remove_file(&path).await.ok();
    }

    #[test]
    fn aggregates_steps_and_cost_per_task() {
        let entries = vec![
            entry("t1", "dev-back", 1, false, 0.1, 100),
            entry("t1", "dev-back", 2, true, 0.4, 200),
            entry("t2", "dev-back", 1, true, 0.1, 100),
            entry("t3", "dev-front", 1, true, 1.0, 50),
        ];
        let stats = aggregate(&entries, Some("dev-back"));
        assert_eq!(stats.len(), 1);
        let s = &stats[0];
        assert_eq!(s.resolved_phases, 2);
        assert_eq!(s.by_step.get(&1), Some(&1));
        assert_eq!(s.by_step.get(&2), Some(&1));
        assert!((s.total_cost_usd - 0.6).abs() < 1e-9);
        assert!((s.avg_cost_usd_per_task - 0.3).abs() < 1e-9);

        let all = aggregate(&entries, None);
        assert_eq!(all.len(), 2);
    }
}
