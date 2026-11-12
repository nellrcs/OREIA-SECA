//! Resumo de uso para a área **Uso** do painel: tokens, custo, taxa de
//! sucesso e duração, a partir de `data/escalation.jsonl` (uma linha por
//! tentativa de um agente — o mesmo arquivo que alimenta `/stats`).
//!
//! Lê o arquivo direto do disco em vez de perguntar à API: assim a área
//! funciona com o orquestrador parado, e o histórico inteiro fica disponível
//! (a API só devolve o agregado por papel). [`summarize`] é pura — recebe as
//! entradas e o "agora" — para dar pra testar sem relógio nem disco.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};

use crate::executor::log::EscalationLogEntry;

/// Quantos dias a série "tokens por dia" cobre (o mais antigo primeiro).
pub const DAILY_DAYS: usize = 14;
/// Quantas tentativas recentes a tabela "últimas execuções" guarda.
pub const RECENT_LEN: usize = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Window {
    pub attempts: usize,
    pub passed: usize,
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupUsage {
    /// Nome do modelo ou do papel.
    pub name: String,
    pub attempts: usize,
    pub passed: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub avg_duration_ms: u64,
    pub cost_usd: f64,
}

impl GroupUsage {
    pub fn success_pct(&self) -> f64 {
        pct(self.passed, self.attempts)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecentAttempt {
    pub at: DateTime<Utc>,
    pub role: String,
    pub model: String,
    pub executor: String,
    pub step: usize,
    pub passed: bool,
    pub duration_ms: u64,
    pub tokens: u64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSummary {
    /// De onde veio (`None` = nenhum `escalation.jsonl` encontrado ainda).
    pub source: Option<PathBuf>,
    pub attempts: usize,
    pub passed: usize,
    /// Tarefas distintas (`task_id`).
    pub tasks: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
    pub avg_duration_ms: u64,
    pub last_24h: Window,
    pub last_7d: Window,
    /// Mais usado primeiro.
    pub by_model: Vec<GroupUsage>,
    pub by_role: Vec<GroupUsage>,
    /// [`DAILY_DAYS`] dias, do mais antigo ao de hoje, com zero onde não houve uso.
    pub daily_tokens: Vec<(NaiveDate, u64)>,
    /// Mais recente primeiro.
    pub recent: Vec<RecentAttempt>,
}

impl UsageSummary {
    pub fn is_empty(&self) -> bool {
        self.attempts == 0
    }

    pub fn success_pct(&self) -> f64 {
        pct(self.passed, self.attempts)
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

fn pct(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn tokens_of(e: &EscalationLogEntry) -> u64 {
    u64::from(e.prompt_tokens) + u64::from(e.completion_tokens)
}

#[derive(Default)]
struct GroupAcc {
    attempts: usize,
    passed: usize,
    prompt: u64,
    completion: u64,
    duration: u64,
    cost: f64,
}

fn group_by<'a>(entries: &'a [EscalationLogEntry], key: impl Fn(&'a EscalationLogEntry) -> &'a str) -> Vec<GroupUsage> {
    let mut acc: BTreeMap<&str, GroupAcc> = BTreeMap::new();
    for e in entries {
        let g = acc.entry(key(e)).or_default();
        g.attempts += 1;
        g.passed += usize::from(e.passed);
        g.prompt += u64::from(e.prompt_tokens);
        g.completion += u64::from(e.completion_tokens);
        g.duration += e.duration_ms;
        g.cost += e.cost_usd;
    }
    let mut groups: Vec<GroupUsage> = acc
        .into_iter()
        .map(|(name, g)| GroupUsage {
            name: name.to_string(),
            attempts: g.attempts,
            passed: g.passed,
            prompt_tokens: g.prompt,
            completion_tokens: g.completion,
            avg_duration_ms: g.duration.checked_div(g.attempts as u64).unwrap_or(0),
            cost_usd: g.cost,
        })
        .collect();
    groups.sort_by(|a, b| b.attempts.cmp(&a.attempts).then(a.name.cmp(&b.name)));
    groups
}

fn window(entries: &[EscalationLogEntry], since: DateTime<Utc>) -> Window {
    let mut w = Window::default();
    for e in entries.iter().filter(|e| e.at >= since) {
        w.attempts += 1;
        w.passed += usize::from(e.passed);
        w.tokens += tokens_of(e);
        w.cost_usd += e.cost_usd;
    }
    w
}

pub fn summarize(entries: &[EscalationLogEntry], now: DateTime<Utc>) -> UsageSummary {
    let tasks: BTreeSet<&str> = entries.iter().map(|e| e.task_id.as_str()).collect();
    let total_duration: u64 = entries.iter().map(|e| e.duration_ms).sum();

    let today = now.date_naive();
    let mut per_day: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    for e in entries {
        *per_day.entry(e.at.date_naive()).or_insert(0) += tokens_of(e);
    }
    let daily_tokens = (0..DAILY_DAYS as i64)
        .rev()
        .map(|back| {
            let day = today - Duration::days(back);
            (day, per_day.get(&day).copied().unwrap_or(0))
        })
        .collect();

    let mut recent: Vec<&EscalationLogEntry> = entries.iter().collect();
    recent.sort_by_key(|e| std::cmp::Reverse(e.at));
    let recent = recent
        .into_iter()
        .take(RECENT_LEN)
        .map(|e| RecentAttempt {
            at: e.at,
            role: e.role.clone(),
            model: e.model.clone(),
            executor: e.executor.clone(),
            step: e.step,
            passed: e.passed,
            duration_ms: e.duration_ms,
            tokens: tokens_of(e),
            reason: e.reason.clone(),
        })
        .collect();

    UsageSummary {
        source: None,
        attempts: entries.len(),
        passed: entries.iter().filter(|e| e.passed).count(),
        tasks: tasks.len(),
        prompt_tokens: entries.iter().map(|e| u64::from(e.prompt_tokens)).sum(),
        completion_tokens: entries.iter().map(|e| u64::from(e.completion_tokens)).sum(),
        cost_usd: entries.iter().map(|e| e.cost_usd).sum(),
        avg_duration_ms: total_duration.checked_div(entries.len() as u64).unwrap_or(0),
        last_24h: window(entries, now - Duration::hours(24)),
        last_7d: window(entries, now - Duration::days(7)),
        by_model: group_by(entries, |e| e.model.as_str()),
        by_role: group_by(entries, |e| e.role.as_str()),
        daily_tokens,
        recent,
    }
}

/// Onde o `escalation.jsonl` pode estar, em ordem: o bind mount do
/// `docker-compose.yml` (`volumes/oreia-seca/data`, o caso normal) e depois
/// `[paths].data` relativo à pasta do projeto (orquestrador rodando nativo).
pub fn candidate_paths(paths_data: &str) -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("volumes/oreia-seca/data/escalation.jsonl")];
    let data = if paths_data.trim().is_empty() { "data" } else { paths_data.trim() };
    let native = Path::new(data).join("escalation.jsonl");
    if !v.contains(&native) {
        v.push(native);
    }
    v
}

fn read_entries(path: &Path) -> Option<Vec<EscalationLogEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
}

/// Lê o primeiro candidato que existir. Nenhum existir não é erro: é só uma
/// instalação que ainda não rodou nenhuma tarefa.
pub fn load(candidates: &[PathBuf], now: DateTime<Utc>) -> UsageSummary {
    for path in candidates {
        if let Some(entries) = read_entries(path) {
            let mut summary = summarize(&entries, now);
            summary.source = Some(path.clone());
            return summary;
        }
    }
    UsageSummary::default()
}

/// `1234567` → `1,2M`; `45300` → `45,3k`; `812` → `812`.
pub fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0).replace('.', ",")
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0).replace('.', ",")
    } else {
        n.to_string()
    }
}

/// `950` → `950ms`; `12_300` → `12s`; `185_000` → `3m05s`; `3_900_000` → `1h05m`.
pub fn human_duration(ms: u64) -> String {
    let secs = ms / 1000;
    if ms < 1000 {
        format!("{ms}ms")
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Custo em dólar; execuções em modelo local custam zero e aparecem como `—`.
pub fn human_cost(usd: f64) -> String {
    if usd <= 0.0 {
        "—".to_string()
    } else if usd < 0.01 {
        "< $0,01".to_string()
    } else {
        format!("${usd:.2}").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn entry(
        task: &str,
        role: &str,
        model: &str,
        passed: bool,
        tokens: (u32, u32),
        cost: f64,
        ms: u64,
        hours_ago: i64,
    ) -> EscalationLogEntry {
        EscalationLogEntry {
            task_id: task.into(),
            role: role.into(),
            phase: 1,
            step: 1,
            executor: "builtin".into(),
            model: model.into(),
            passed,
            reason: (!passed).then(|| "testes falharam".to_string()),
            duration_ms: ms,
            prompt_tokens: tokens.0,
            completion_tokens: tokens.1,
            cost_usd: cost,
            at: now() - Duration::hours(hours_ago),
            handoff_from: None,
        }
    }

    fn sample() -> Vec<EscalationLogEntry> {
        vec![
            entry("t1", "dev-back", "qwen", false, (1000, 200), 0.0, 10_000, 1),
            entry("t1", "dev-back", "qwen", true, (2000, 400), 0.0, 20_000, 2),
            entry("t2", "dev-front", "sonnet", true, (3000, 600), 0.5, 30_000, 30),
            entry("t3", "dev-back", "qwen", true, (500, 100), 0.0, 6_000, 24 * 10),
        ]
    }

    #[test]
    fn totals_count_attempts_tasks_tokens_and_cost() {
        let s = summarize(&sample(), now());
        assert_eq!(s.attempts, 4);
        assert_eq!(s.passed, 3);
        assert_eq!(s.tasks, 3);
        assert_eq!(s.prompt_tokens, 6500);
        assert_eq!(s.completion_tokens, 1300);
        assert_eq!(s.total_tokens(), 7800);
        assert!((s.cost_usd - 0.5).abs() < 1e-9);
        assert_eq!(s.avg_duration_ms, 16_500);
        assert!((s.success_pct() - 75.0).abs() < 1e-9);
    }

    #[test]
    fn windows_only_count_what_is_recent_enough() {
        let s = summarize(&sample(), now());
        assert_eq!(s.last_24h.attempts, 2);
        assert_eq!(s.last_24h.passed, 1);
        assert_eq!(s.last_24h.tokens, 3600);
        assert_eq!(s.last_7d.attempts, 3);
        assert!((s.last_7d.cost_usd - 0.5).abs() < 1e-9);
    }

    #[test]
    fn groups_by_model_and_role_most_used_first() {
        let s = summarize(&sample(), now());
        assert_eq!(s.by_model[0].name, "qwen");
        assert_eq!(s.by_model[0].attempts, 3);
        assert_eq!(s.by_model[0].passed, 2);
        assert_eq!(s.by_model[0].avg_duration_ms, 12_000);
        assert_eq!(s.by_model[1].name, "sonnet");
        assert!((s.by_model[1].success_pct() - 100.0).abs() < 1e-9);
        assert_eq!(s.by_role[0].name, "dev-back");
        assert_eq!(s.by_role[1].name, "dev-front");
    }

    #[test]
    fn daily_series_is_zero_filled_and_ends_today() {
        let s = summarize(&sample(), now());
        assert_eq!(s.daily_tokens.len(), DAILY_DAYS);
        assert_eq!(s.daily_tokens.last().unwrap().0, now().date_naive());
        assert_eq!(s.daily_tokens.last().unwrap().1, 3600, "as duas tentativas de hoje");
        let yesterday = &s.daily_tokens[DAILY_DAYS - 2];
        assert_eq!(yesterday.1, 3600, "a tentativa de 30h atrás");
        assert_eq!(s.daily_tokens.iter().filter(|(_, t)| *t == 0).count(), DAILY_DAYS - 3);
    }

    #[test]
    fn recent_is_newest_first_and_keeps_the_failure_reason() {
        let s = summarize(&sample(), now());
        assert_eq!(s.recent.len(), 4);
        assert!(!s.recent[0].passed);
        assert_eq!(s.recent[0].reason.as_deref(), Some("testes falharam"));
        assert!(s.recent[0].at > s.recent[1].at);
    }

    #[test]
    fn empty_log_gives_an_empty_summary_without_dividing_by_zero() {
        let s = summarize(&[], now());
        assert!(s.is_empty());
        assert_eq!(s.avg_duration_ms, 0);
        assert_eq!(s.success_pct(), 0.0);
        assert_eq!(s.daily_tokens.len(), DAILY_DAYS);
    }

    #[test]
    fn load_reads_the_first_existing_candidate_and_skips_broken_lines() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-usage-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("escalation.jsonl");
        let line = serde_json::to_string(&sample()[0]).unwrap();
        std::fs::write(&path, format!("{line}\nisto não é json\n\n{line}\n")).unwrap();

        let s = load(&[dir.join("nao-existe.jsonl"), path.clone()], now());
        assert_eq!(s.attempts, 2);
        assert_eq!(s.source, Some(path));

        assert!(load(&[dir.join("nao-existe.jsonl")], now()).source.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn candidate_paths_prefers_the_compose_volume_then_the_native_data_dir() {
        let v = candidate_paths("data");
        assert_eq!(v[0], PathBuf::from("volumes/oreia-seca/data/escalation.jsonl"));
        assert_eq!(v[1], PathBuf::from("data").join("escalation.jsonl"));
        assert_eq!(candidate_paths("  ").len(), 2);
    }

    #[test]
    fn human_formats() {
        assert_eq!(human_tokens(812), "812");
        assert_eq!(human_tokens(45_300), "45,3k");
        assert_eq!(human_tokens(1_234_567), "1,2M");
        assert_eq!(human_duration(950), "950ms");
        assert_eq!(human_duration(12_300), "12s");
        assert_eq!(human_duration(185_000), "3m05s");
        assert_eq!(human_duration(3_900_000), "1h05m");
        assert_eq!(human_cost(0.0), "—");
        assert_eq!(human_cost(0.004), "< $0,01");
        assert_eq!(human_cost(1.5), "$1,50");
    }
}
