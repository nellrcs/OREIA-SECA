//! Planejador: pede ao modelo um JSON com fases e valida a resposta.

use oreia_core::{ChatMessage, Phase, PhaseStatus};
use oreia_llm::{GenerateOpts, ModelRouter};
use serde::Deserialize;

#[derive(Deserialize)]
struct RawPlan {
    phases: Vec<RawPhase>,
}

#[derive(Deserialize)]
struct RawPhase {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
}

/// Extrai o primeiro objeto JSON de uma resposta que pode ter texto em volta.
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Some(v);
    }
    let stripped = t.replace("```json", "").replace("```", "");
    if let Ok(v) = serde_json::from_str(stripped.trim()) {
        return Some(v);
    }
    let (start, end) = (t.find('{')?, t.rfind('}')?);
    (start < end).then(|| serde_json::from_str(&t[start..=end]).ok()).flatten()
}

pub fn validate(value: serde_json::Value, max_phases: usize) -> anyhow::Result<Vec<Phase>> {
    let raw: RawPlan = serde_json::from_value(value).map_err(|e| anyhow::anyhow!("resposta sem \"phases\": {e}"))?;
    anyhow::ensure!(!raw.phases.is_empty(), "plano sem fases");

    raw.phases
        .into_iter()
        .take(max_phases)
        .enumerate()
        .map(|(i, p)| {
            let name = p.name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let instruction = p.instruction.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            match (name, instruction) {
                (Some(name), Some(instruction)) => Ok(Phase {
                    id: p.id.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("p{}", i + 1)),
                    name,
                    instruction,
                    status: PhaseStatus::Pending,
                    summary: None,
                    resolved_step: None,
                }),
                _ => anyhow::bail!("fase {} sem name ou instruction", i + 1),
            }
        })
        .collect()
}

pub async fn plan(router: &ModelRouter, goal: &str, system: &str, max_phases: usize) -> anyhow::Result<Vec<Phase>> {
    const ATTEMPTS: usize = 2;
    let mut messages = vec![ChatMessage::user(format!("Task: {goal}"))];
    let opts = GenerateOpts::system(system).max_tokens(2048);
    let mut last = anyhow::anyhow!("sem tentativas");

    for attempt in 1..=ATTEMPTS {
        let c = router.generate("planner", &messages, &opts).await?;
        match extract_json(&c.text)
            .ok_or_else(|| anyhow::anyhow!("resposta sem JSON"))
            .and_then(|v| validate(v, max_phases))
        {
            Ok(phases) => return Ok(phases),
            Err(e) => {
                tracing::warn!(attempt, error = %e, "planner devolveu plano inválido");
                messages.push(ChatMessage::assistant(c.text));
                messages.push(ChatMessage::user(format!(
                    "Invalid plan ({e}). Reply ONLY with the JSON object in the required format."
                )));
                last = e;
            }
        }
    }
    anyhow::bail!("o planejador falhou após {ATTEMPTS} tentativas: {last}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_noise() {
        assert!(extract_json(r#"{"phases":[]}"#).is_some());
        assert!(extract_json("```json\n{\"phases\":[]}\n```").is_some());
        assert!(extract_json("Claro! Aqui está: {\"phases\": []} Espero ajudar").is_some());
        assert!(extract_json("nada aqui").is_none());
    }

    #[test]
    fn validates_and_fills_ids() {
        let v = serde_json::json!({"phases":[
            {"name":"A","instruction":"do a"},
            {"id":"x","name":"B","instruction":"do b"}
        ]});
        let p = validate(v, 8).unwrap();
        assert_eq!(p[0].id, "p1");
        assert_eq!(p[1].id, "x");
    }

    #[test]
    fn caps_and_rejects() {
        let many: Vec<_> = (0..12).map(|i| serde_json::json!({"name": format!("f{i}"), "instruction": "x"})).collect();
        assert_eq!(validate(serde_json::json!({"phases": many}), 8).unwrap().len(), 8);
        assert!(validate(serde_json::json!({"phases": []}), 8).is_err());
        assert!(validate(serde_json::json!({"phases": [{"name": "sem instrução"}]}), 8).is_err());
        assert!(validate(serde_json::json!({"fases": []}), 8).is_err());
    }
}
