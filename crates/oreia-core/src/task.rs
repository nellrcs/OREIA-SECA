use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::message::{Source, WorkItem};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Planned,
    AwaitingApproval,
    Running,
    Done,
    DoneWithErrors,
    Rejected,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Planned => "planned",
            TaskStatus::AwaitingApproval => "awaiting_approval",
            TaskStatus::Running => "running",
            TaskStatus::Done => "done",
            TaskStatus::DoneWithErrors => "done_with_errors",
            TaskStatus::Rejected => "rejected",
            TaskStatus::Failed => "failed",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::DoneWithErrors | TaskStatus::Rejected | TaskStatus::Failed)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    #[default]
    Pending,
    Done,
    Failed,
}

/// Qual degrau da escada de escalada resolveu a fase (veja `oreia-seca::executor`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepResolution {
    /// Posição do degrau na lista `[escalation.<papel>].steps` (1-based).
    pub step: usize,
    pub executor: String,
    pub model: String,
}

/// Uma etapa do plano gerado pelo planejador.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub instruction: String,
    #[serde(default)]
    pub status: PhaseStatus,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub resolved_step: Option<StepResolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub source: Source,
    pub user_id: String,
    pub goal: String,
    #[serde(default)]
    pub work_item: Option<WorkItem>,
    #[serde(default)]
    pub model_role: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub phases: Vec<Phase>,
    #[serde(default)]
    pub pull_request: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(source: Source, user_id: impl Into<String>, goal: impl Into<String>) -> Self {
        let now = Utc::now();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        Self {
            id: format!("task-{}", &uid[..8]),
            source,
            user_id: user_id.into(),
            goal: goal.into(),
            work_item: None,
            model_role: None,
            status: TaskStatus::Planned,
            phases: Vec::new(),
            pull_request: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    pub fn plan_text(&self) -> String {
        self.phases.iter().enumerate().map(|(i, p)| format!("{}. {}", i + 1, p.name)).collect::<Vec<_>>().join("\n")
    }

    pub fn done_phases(&self) -> impl Iterator<Item = &Phase> {
        self.phases.iter().filter(|p| p.status == PhaseStatus::Done)
    }

    pub fn failed_count(&self) -> usize {
        self.phases.iter().filter(|p| p.status == PhaseStatus::Failed).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_task_has_short_id() {
        let t = Task::new(Source::Telegram, "1", "fazer algo");
        assert!(t.id.starts_with("task-"));
        assert_eq!(t.id.len(), "task-".len() + 8);
        assert_eq!(t.status, TaskStatus::Planned);
    }

    #[test]
    fn serde_roundtrip_with_defaults() {
        let json = r#"{
            "id":"task-1","source":"rest","user_id":"cron","goal":"g",
            "status":"planned",
            "phases":[{"id":"p1","name":"A","instruction":"faz A"}],
            "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
        }"#;
        let t: Task = serde_json::from_str(json).unwrap();
        assert_eq!(t.phases[0].status, PhaseStatus::Pending);
        assert_eq!(t.plan_text(), "1. A");
    }
}
