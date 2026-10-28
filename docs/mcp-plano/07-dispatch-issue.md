# Etapa 7 — `dispatch_issue`

[← 06](06-review-pr-com-limite.md) · [checklist](00-checklist.md) · próxima: [08](08-http-opcional.md)

## Objetivo

Atalho pra via automática: o P.O. decide, pelo MCP, mandar a issue pra um
agente (igual ao `/assign` do Telegram), com a opção de forçar um degrau
específico da escada de escalada (ex. pular direto pro degrau pago).

## Arquivos tocados

- `crates/oreia-seca/src/maker/mcp.rs` — `dispatch_issue(repo, n, agent, step: Option<usize>)`.
- `crates/oreia-seca/src/maker/exec.rs` — `PhaseCtx`/`run_escalation` ganham
  um `Option<usize>` (degrau 1-based) opcional: quando presente, a primeira
  tentativa da fase começa nesse degrau em vez do 1; escala normalmente a
  partir dali se ele falhar. Olhar o corpo de `run_phase_ladder`/`run_escalation`
  nesta etapa pra decidir o encaixe exato (provavelmente um campo a mais em
  `Task` ou um parâmetro passado só na primeira chamada de `execute_inner`
  pra esta tarefa específica).

## Decisões de design

- Reaproveita `assign_and_reply` (resolver label do agente + `add_issue_labels`)
  de `maker/commands.rs` — mesma extração de helper compartilhado já
  combinada na etapa 4 (`resolve_agent_label`).
- **Respeita `ask_before_step`/`max_cost_usd`**: não contornar essas
  travas — `dispatch_issue` só FORÇA o degrau inicial da escalada, a
  aprovação (`EscalationConfig.ask_before_step`) e o teto de custo
  (`max_cost_usd`) continuam valendo normalmente a partir desse ponto.

## Testes obrigatórios

- `dispatch_issue` sem `step`: equivalente a `/assign` (label aplicada,
  webhook dispara o agente).
- `dispatch_issue` com `step`: a tarefa começa no degrau pedido (verificar via
  `data/escalation.jsonl`, campo `step`), e ainda assim respeita
  `ask_before_step`/`max_cost_usd` se o degrau pedido já estiver além deles.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [ ] `dispatch_issue` funcionando com e sem `step` forçado.
- [ ] Checklist atualizado com o commit.
