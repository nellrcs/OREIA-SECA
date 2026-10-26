//! O orquestrador ("Maker", como no OREIA-SECA).
//!
//! Recebe mensagens de qualquer canal, decide entre resposta direta e tarefa
//! planejada, pede aprovação quando a fonte exige, coloca a execução na fila
//! e devolve o progresso para quem pediu.

mod commands;
mod direct;
mod exec;
mod forge;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use oreia_channels::{Channel, Recipient};
use oreia_core::actions::ActionRegistry;
use oreia_core::approval::{ApprovalBroker, ApprovalOutcome};
use oreia_core::queue::TaskQueue;
use oreia_core::session::SessionStore;
use oreia_core::store::TaskStore;
use oreia_core::{intent, Incoming, Source, Task, TaskStatus, WorkItem};
use oreia_llm::ModelRouter;

use crate::config::Config;
use crate::prompts;

pub use forge::ForgejoCtx;

pub struct Maker {
    pub cfg: Arc<Config>,
    pub router: Arc<ModelRouter>,
    pub queue: TaskQueue,
    pub approvals: ApprovalBroker,
    pub store: TaskStore,
    pub sessions: SessionStore,
    pub forgejo: Option<ForgejoCtx>,
    actions: RwLock<Arc<ActionRegistry>>,
    channels: Vec<Arc<dyn Channel>>,
    /// Itens do Forgejo em andamento (`dono/repo#issue`), para não duplicar.
    active_work: Mutex<HashSet<String>>,
    /// Próxima mensagem de TEXTO LIVRE esperada por sessão, nos assistentes de
    /// botão (`/issue`, `/assign`, `/prs` → "Pedir ajuste") — ex.: o título de
    /// uma issue, que não dá pra virar botão. Ver [`commands::Pending`].
    pending: Mutex<HashMap<String, commands::Pending>>,
}

impl Maker {
    pub async fn new(
        cfg: Arc<Config>,
        router: Arc<ModelRouter>,
        channels: Vec<Arc<dyn Channel>>,
        forgejo: Option<ForgejoCtx>,
    ) -> anyhow::Result<Arc<Self>> {
        tokio::fs::create_dir_all(&cfg.paths.workspace).await?;
        let store = TaskStore::open(cfg.tasks_dir()).await?;
        let sessions = SessionStore::open(cfg.sessions_file())?;

        let mut actions = ActionRegistry::with_builtins();
        let n = actions.load_skills(&cfg.paths.skills);
        tracing::info!(skills = n, total = actions.len(), "ações carregadas");

        Ok(Arc::new(Self {
            queue: TaskQueue::new(cfg.queue.concurrency, cfg.queue_timeout()),
            approvals: ApprovalBroker::new(),
            store,
            sessions,
            forgejo,
            actions: RwLock::new(Arc::new(actions)),
            channels,
            active_work: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
            router,
            cfg,
        }))
    }

    pub fn actions(&self) -> Arc<ActionRegistry> {
        self.actions.read().expect("lock de ações").clone()
    }

    pub fn reload_skills(&self) -> usize {
        let mut reg = ActionRegistry::with_builtins();
        let n = reg.load_skills(&self.cfg.paths.skills);
        *self.actions.write().expect("lock de ações") = Arc::new(reg);
        n
    }

    fn human_channels(&self) -> impl Iterator<Item = &Arc<dyn Channel>> {
        self.channels.iter().filter(|c| c.source().is_human())
    }

    /// Responde a quem mandou. Fontes sem humano (REST, cron, Forgejo) vão para todos os canais humanos.
    pub async fn reply(&self, source: Source, user_id: &str, text: &str) {
        if source.is_human() {
            if let Some(ch) = self.channels.iter().find(|c| c.source() == source) {
                if let Err(e) = ch.send(&Recipient::user(user_id), text).await {
                    tracing::warn!(channel = ch.name(), error = %e, "falha ao responder");
                }
                return;
            }
        }
        self.broadcast(text).await;
    }

    pub async fn broadcast(&self, text: &str) {
        for ch in self.human_channels() {
            if let Err(e) = ch.send(&Recipient::Broadcast, text).await {
                tracing::warn!(channel = ch.name(), error = %e, "falha no broadcast");
            }
        }
    }

    /// Pergunta de múltipla escolha a quem mandou (botões no Telegram; lista
    /// numerada em texto puro nos canais sem suporte).
    pub async fn pick(&self, source: Source, user_id: &str, text: &str, options: &[(String, String)]) {
        if source.is_human() {
            if let Some(ch) = self.channels.iter().find(|c| c.source() == source) {
                if let Err(e) = ch.ask_pick(&Recipient::user(user_id), text, options).await {
                    tracing::warn!(channel = ch.name(), error = %e, "falha ao perguntar");
                }
                return;
            }
        }
        let lines: Vec<String> = options.iter().map(|(label, cmd)| format!("• {label} → {cmd}")).collect();
        self.broadcast(&format!("{text}\n{}", lines.join("\n"))).await;
    }

    async fn typing(&self, source: Source, user_id: &str) {
        if let Some(ch) = self.channels.iter().find(|c| c.source() == source) {
            ch.typing(&Recipient::user(user_id)).await;
        }
    }

    /// Ponto de entrada de toda mensagem.
    pub async fn handle(self: Arc<Self>, msg: Incoming) {
        tracing::info!(source = %msg.source, user = %msg.user_id, text = %msg.text.chars().take(80).collect::<String>(), "mensagem");
        if let Err(e) = self.clone().handle_inner(&msg).await {
            tracing::error!(error = %e, "erro ao tratar mensagem");
            self.reply(msg.source, &msg.user_id, &format!("❌ Erro: {e:#}")).await;
        }
    }

    async fn handle_inner(self: Arc<Self>, msg: &Incoming) -> anyhow::Result<()> {
        let text = msg.text.trim();
        if let Some(cmd) = text.strip_prefix('/') {
            // Qualquer comando novo abandona um assistente de botão em aberto — ele já
            // carrega tudo que precisa nos próprios argumentos, não depende do pendente.
            self.pending.lock().expect("lock").remove(&msg.session_key());
            return self.command(msg, cmd).await;
        }
        let pending = self.pending.lock().expect("lock").remove(&msg.session_key());
        if let Some(p) = pending {
            return self.resolve_pending(msg, p, text).await;
        }
        if msg.work_item.is_some() || intent::is_complex(text) {
            return self.start_task(msg, text).await;
        }
        self.reply_direct(msg).await
    }

    /// Planeja, pede aprovação se preciso e coloca a execução na fila.
    pub async fn start_task(self: Arc<Self>, msg: &Incoming, goal: &str) -> anyhow::Result<()> {
        let result = self.clone().start_task_inner(msg, goal).await;
        if result.is_err() {
            if let Some(w) = &msg.work_item {
                self.release_work(w);
            }
        }
        result
    }

    async fn start_task_inner(self: Arc<Self>, msg: &Incoming, goal: &str) -> anyhow::Result<()> {
        self.typing(msg.source, &msg.user_id).await;

        let agent_prompt = msg.work_item.as_ref().and_then(|w| self.forgejo.as_ref()?.agent_prompt(&w.agent));
        let planner_model = self.router.active_model("planner").unwrap_or_else(|| "?".into());
        self.reply(msg.source, &msg.user_id, &format!("🧠 Planejando com {planner_model}...")).await;

        let system =
            prompts::planner(&self.actions().prompt_block(), agent_prompt.as_deref(), self.cfg.executor.max_phases);
        let phases = crate::planner::plan(&self.router, goal, &system, self.cfg.executor.max_phases)
            .await
            .map_err(|e| anyhow::anyhow!("não consegui planejar: {e:#}"))?;

        let mut task = Task::new(msg.source, &msg.user_id, goal);
        task.phases = phases;
        task.work_item = msg.work_item.clone();
        task.model_role = msg.work_item.as_ref().and_then(|w| self.cfg.agent(&w.agent)).map(|a| a.role().to_string());
        self.store.save(&task).await?;

        let header = match &task.work_item {
            Some(w) => format!("{} · {} #{}", w.agent, w.repo_full_name(), w.issue),
            None => format!("via {}", task.source),
        };
        let plan_msg = format!(
            "📋 Tarefa {} ({header})\n\"{}\"\n\nPlano ({} fases):\n{}",
            task.id,
            goal.chars().take(160).collect::<String>(),
            task.phases.len(),
            task.plan_text()
        );

        if self.cfg.needs_approval(task.source) {
            task.set_status(TaskStatus::AwaitingApproval);
            self.store.save(&task).await?;
            for ch in self.human_channels() {
                if let Err(e) = ch.ask_approval(&task.id, &format!("🔔 Aprovação necessária\n\n{plan_msg}")).await
                {
                    tracing::warn!(channel = ch.name(), error = %e, "falha ao pedir aprovação");
                }
            }
            let timeout = Duration::from_secs(self.cfg.approval.timeout_secs);
            let outcome = self.approvals.request(&task.id, &plan_msg, timeout).await;
            let note = match outcome {
                ApprovalOutcome::Approved => format!("✅ {} aprovada, iniciando.", task.id),
                ApprovalOutcome::Rejected => format!("❌ {} rejeitada.", task.id),
                ApprovalOutcome::Expired => format!("⌛ {} expirou sem resposta.", task.id),
            };
            for ch in self.human_channels() {
                ch.approval_resolved(&task.id, &note).await;
            }
            if outcome != ApprovalOutcome::Approved {
                task.set_status(TaskStatus::Rejected);
                self.store.save(&task).await?;
                if let Some(w) = &task.work_item {
                    self.release_work(w);
                }
                return Ok(());
            }
        } else {
            let model = self
                .router
                .active_model(task.model_role.as_deref().unwrap_or("executor"))
                .unwrap_or_else(|| "?".into());
            self.reply(msg.source, &msg.user_id, &format!("{plan_msg}\n\n🤖 Modelo: {model}")).await;
        }

        self.enqueue(task);
        Ok(())
    }

    /// Coloca a tarefa na fila de execução.
    pub fn enqueue(self: &Arc<Self>, task: Task) {
        let label = task.goal.chars().take(40).collect::<String>();
        let (source, user) = (task.source, task.user_id.clone());
        let me = self.clone();
        let _handle = self.queue.spawn(label, async move { me.execute(task).await });

        let queued = self.queue.status().queued;
        if queued > 0 {
            let me = self.clone();
            tokio::spawn(async move {
                me.reply(source, &user, &format!("⏳ Na fila (posição {queued}).")).await;
            });
        }
    }

    pub(crate) fn work_key(w: &WorkItem) -> String {
        format!("{}#{}", w.repo_full_name(), w.issue)
    }

    /// Marca o item como ativo. Retorna `false` se já havia um agente nele.
    pub(crate) fn claim_work(&self, w: &WorkItem) -> bool {
        self.active_work.lock().expect("lock").insert(Self::work_key(w))
    }

    pub(crate) fn release_work(&self, w: &WorkItem) {
        self.active_work.lock().expect("lock").remove(&Self::work_key(w));
    }

    pub(crate) fn is_active(&self, w: &WorkItem) -> bool {
        self.active_work.lock().expect("lock").contains(&Self::work_key(w))
    }

    /// Tarefas que estavam rodando quando o processo caiu.
    pub async fn recover_interrupted(&self) {
        match self.store.list_by_status(TaskStatus::Running).await {
            Ok(list) if !list.is_empty() => {
                let ids: Vec<String> = list
                    .iter()
                    .map(|t| format!("• {} — {}", t.id, t.goal.chars().take(50).collect::<String>()))
                    .collect();
                self.broadcast(&format!(
                    "⚠️ Tarefas interrompidas no último desligamento:\n{}\n\nUse /retry <id> para retomar.",
                    ids.join("\n")
                ))
                .await;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "não consegui listar tarefas interrompidas"),
        }
    }

    pub fn status_json(&self) -> serde_json::Value {
        serde_json::json!({
            "queue": self.queue.status(),
            "pending_approvals": self.approvals.list_pending().into_iter().map(|(id, q)| serde_json::json!({"id": id, "question": q})).collect::<Vec<_>>(),
            "roles": self.router.describe(),
            "models": self.router.describe_models(),
            "active_work": self.active_work.lock().expect("lock").iter().cloned().collect::<Vec<_>>(),
            "agents": self.forgejo.as_ref().map(|f| f.agent_names()).unwrap_or_default(),
        })
    }

    pub async fn shutdown(&self) {
        if let Err(e) = self.sessions.flush() {
            tracing::warn!(error = %e, "falha ao salvar sessões");
        }
        if !self.queue.drain(Duration::from_secs(10)).await {
            tracing::warn!("encerrando com tarefas em andamento; use /retry depois");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
