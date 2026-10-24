//! Executor embutido: o loop agêntico original (modelo → ações XML → resultado),
//! agora atrás da interface [`super::PhaseExecutor`]. Um papel sem
//! `[escalation.<papel>]` usa só este executor, num degrau único com o modelo
//! do próprio papel — ou seja, o comportamento é idêntico ao de antes desta
//! interface existir.

use std::sync::Arc;

use async_trait::async_trait;
use oreia_core::actions::{ActionContext, ActionRegistry, RunResult};
use oreia_core::session::estimate_tokens;
use oreia_core::ChatMessage;
use oreia_llm::{GenerateOpts, ModelRouter};

use super::{FailureContext, ModelSelector, PhaseExecutor, PhaseOutcome, PhaseRequest};
use crate::prompts;

pub struct BuiltinExecutor {
    router: Arc<ModelRouter>,
    actions: Arc<ActionRegistry>,
    max_steps: usize,
}

impl BuiltinExecutor {
    pub fn new(router: Arc<ModelRouter>, actions: Arc<ActionRegistry>, max_steps: usize) -> Self {
        Self { router, actions, max_steps }
    }
}

fn failure_block(f: &FailureContext) -> String {
    let mut s = format!("\n\nPREVIOUS ATTEMPT FAILED: {}\n", f.error);
    if !f.test_output.is_empty() {
        s.push_str(&format!("\nTest output (tail):\n{}\n", f.test_output));
    }
    if !f.diff.is_empty() {
        s.push_str(&format!("\nAttempted diff (truncated):\n{}\n", f.diff));
    }
    s
}

#[async_trait]
impl PhaseExecutor for BuiltinExecutor {
    fn name(&self) -> &str {
        "builtin"
    }

    async fn run(&self, req: &PhaseRequest) -> anyhow::Result<PhaseOutcome> {
        let system = prompts::executor(
            &self.actions.prompt_block(),
            req.agent_prompt.as_deref(),
            req.work_item.as_ref(),
            &req.base_branch,
        );
        let ctx = ActionContext {
            task_id: Some(req.task_id.clone()),
            user_id: String::new(),
            workdir: req.workdir.clone(),
            shell_timeout: req.shell_timeout,
        };

        let mut first = format!(
            "GOAL: {}\n\nCOMPLETED PHASES:\n{}\n\nCURRENT PHASE ({}/{}): {}\n{}",
            req.goal, req.completed_summary, req.phase_index, req.phase_total, req.phase_name, req.phase_instruction
        );
        if let Some(f) = &req.failure {
            first.push_str(&failure_block(f));
        }

        let (context, reserve) = match &req.selector {
            ModelSelector::Role(role) => {
                self.router.spec_for(role).map(|s| (s.context as usize, s.reserve_output)).unwrap_or((8_192, 2_048))
            }
            ModelSelector::Pinned { spec, .. } => (spec.context as usize, spec.reserve_output),
        };
        let opts = GenerateOpts::system(&system).max_tokens(reserve);

        let mut messages = vec![ChatMessage::user(first)];
        let mut all = RunResult::default();
        let (mut prompt_tokens, mut completion_tokens) = (0u32, 0u32);

        for _ in 0..self.max_steps.max(1) {
            let c = match &req.selector {
                ModelSelector::Role(role) => self.router.generate(role, &messages, &opts).await?,
                ModelSelector::Pinned { key, .. } => self.router.generate_with_model(key, &messages, &opts).await?,
            };
            prompt_tokens += c.usage.prompt_tokens;
            completion_tokens += c.usage.completion_tokens;

            let run = self.actions.run(&c.text, &ctx).await;
            all.narrative = run.narrative.clone();
            let no_actions = run.results.is_empty();
            let feedback = run.feedback(3_000);
            all.results.extend(run.results);
            if no_actions {
                break;
            }

            messages.push(ChatMessage::assistant(c.text));
            messages.push(ChatMessage::user(format!(
                "Action results:\n{feedback}\n\nIf something failed or is missing for THIS phase, fix it with more actions. \
                 If the phase is complete, reply with only a one-line summary in pt-BR and no actions."
            )));

            let budget = context.saturating_sub(reserve as usize + estimate_tokens(&[ChatMessage::system(&system)]));
            while messages.len() > 3 && estimate_tokens(&messages) > budget {
                messages.drain(1..3);
            }
        }

        let changed_files = super::git_changed_files(&req.workdir).await;
        tracing::debug!(model = req.selector.label(), phase = %req.phase_name, "fase executada pelo builtin");
        Ok(PhaseOutcome {
            summary: all.summary(),
            changed_files,
            prompt_tokens,
            completion_tokens,
            action_errors: all.errors(),
            raw_output: all.narrative.clone(),
        })
    }
}
