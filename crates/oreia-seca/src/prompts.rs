//! Prompts de sistema. Seguem o OREIA-SECA: o plano é escrito em inglês
//! (modelos locais pequenos seguem melhor instruções em inglês) e toda a
//! narrativa para o usuário é em português.

use oreia_core::WorkItem;

pub fn planner(skills: &str, agent_prompt: Option<&str>, max_phases: usize, go_horse: bool) -> String {
    let agent = agent_prompt
        .map(|p| format!("\nTHE EXECUTING AGENT HAS THIS ROLE (plan only work that fits it):\n{p}\n"))
        .unwrap_or_default();
    let verify_rule = if go_horse {
        "GO-HORSE MODE IS ON for this project: NEVER add a phase to run the build, lint or test suite, \
         no matter how risky the change looks. Not even once, not even as the last phase."
    } else {
        "Only add a phase to run the build, lint or test suite when the project already has one AND the \
         change is risky enough that a human can't just eyeball it (logic, state, integration). Skip it for \
         simple, static or cosmetic changes, and always skip it when the goal says no build/test is needed."
    };
    format!(
        r#"You are an expert software development task planner.
IMPORTANT: Write phase names and instructions in English so the executing agent understands them precisely.
Break the goal into small, sequential phases. Each phase must fit in roughly 800 tokens of output.
Never exceed {max_phases} phases. Be specific: name files, functions and commands.
{verify_rule}
{agent}
AVAILABLE ACTIONS AND SKILLS:
{skills}

Respond ONLY with a JSON object, no markdown fences and no text around it:
{{
  "phases": [
    {{ "id": "p1", "name": "Short phase name", "instruction": "Detailed instruction for this phase." }}
  ]
}}"#
    )
}

pub fn executor(
    actions: &str,
    agent_prompt: Option<&str>,
    work: Option<&WorkItem>,
    base_branch: &str,
    go_horse: bool,
) -> String {
    let agent = agent_prompt.map(|p| format!("\nYOUR ROLE:\n{p}\n")).unwrap_or_default();
    let repo = work
        .map(|w| {
            format!(
                "\nREPOSITORY CONTEXT:\n- Repository {} is already cloned in the current directory.\n- You are on branch `{}`, created from `{base_branch}`, for issue #{}.\n- Do NOT run git push, git commit, or open pull requests: the orchestrator does that after your phases.\n- Keep changes focused on the issue.\n",
                w.repo_full_name(),
                // checkout_branch(), não branch_name(): em modo Review é a branch do PR
                // já existente, não a que `branch_name()` calcularia pro reviewer.
                w.checkout_branch(),
                w.issue
            )
        })
        .unwrap_or_default();
    let verify_rule = if go_horse {
        "4. GO-HORSE MODE IS ON for this project: do NOT run any build, lint or test command for any reason — \
         not even if the phase instruction, the goal, or a previous failure asks for it."
    } else {
        "4. Do NOT run build, lint or test commands (npm run build, npm test, cargo test, etc.) unless the \
         current phase instruction explicitly asks for it. Most phases don't need it."
    };
    format!(
        r#"You are an autonomous software developer agent executing ONE phase of a plan.
IMPORTANT: Write all narrative and summaries in Brazilian Portuguese (pt-BR).
{agent}{repo}
AVAILABLE ACTIONS (write the XML exactly like this to run them):

{actions}

RULES:
1. Execute ONLY the current phase.
2. Write complete, working code. No placeholders like "// implement here".
3. You may run several actions in one reply. You will receive their results.
{verify_rule}
5. When the phase is complete, reply WITHOUT actions, with one line in pt-BR summarizing what was done."#
    )
}

pub fn direct(actions: &str) -> String {
    format!(
        r#"Você é um assistente técnico direto e objetivo. Responda sempre em português do Brasil.
Use blocos de código markdown para trechos de código.

Quando precisar inspecionar o sistema (arquivos, processos, git, rede), você pode executar ações com esta sintaxe XML:

{actions}

REGRAS:
1. Para executar ações, responda SOMENTE com as tags XML, sem texto em volta.
2. Ao receber o resultado, responda com um resumo claro em português.
3. Não invente dados: relate apenas resultados reais.
4. Prefira comandos seguros e somente leitura.
5. Se nenhuma ação for necessária, apenas responda em texto."#
    )
}

pub const SUMMARIZE: &str = "Resuma a conversa abaixo em português, em no máximo 15 linhas. \
Preserve decisões, nomes de arquivos, comandos, pendências e preferências do usuário. \
Responda apenas com o resumo.";

#[cfg(test)]
mod tests {
    use super::*;
    use oreia_core::WorkMode;

    #[test]
    fn planner_skips_forced_build_test_phase_language() {
        let p = planner("skills", None, 8, false);
        assert!(!p.contains("The last phase must run"), "{p}");
        assert!(p.contains("no build/test"), "{p}");
        assert!(!p.contains("GO-HORSE"), "{p}");
    }

    #[test]
    fn go_horse_hardens_the_planner_and_executor_prompts() {
        let p = planner("skills", None, 8, true);
        assert!(p.contains("GO-HORSE MODE IS ON"), "{p}");
        assert!(p.contains("Not even once"), "{p}");

        let e = executor("actions", None, None, "main", true);
        assert!(e.contains("GO-HORSE MODE IS ON"), "{e}");
        assert!(!executor("actions", None, None, "main", false).contains("GO-HORSE"));
    }

    #[test]
    fn executor_has_no_repo_context_without_work_item() {
        let s = executor("actions", None, None, "main", false);
        assert!(!s.contains("REPOSITORY CONTEXT"), "{s}");
    }

    /// Regressão: em modo Review o texto dizia pro modelo que ele estava na
    /// branch `agent/reviewer/issue-<pr>` (via `branch_name()`), que nem existe
    /// — a branch de verdade, já com checkout feito, é a do PR original.
    #[test]
    fn executor_shows_the_checked_out_branch_not_the_reviewer_agent_branch() {
        let w = WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 12,
            agent: "reviewer".into(),
            title: String::new(),
            mode: WorkMode::Review,
            review_branch: Some("agent/dev-front/issue-9".into()),
            base_branch: None,
        };
        let s = executor("actions", None, Some(&w), "main", false);
        assert!(s.contains("agent/dev-front/issue-9"), "{s}");
        assert!(!s.contains("agent/reviewer/issue-12"), "{s}");
    }
}
