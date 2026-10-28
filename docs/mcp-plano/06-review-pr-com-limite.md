# Etapa 6 — `review_pr` com limite de rodadas

[← 05](05-checkout-run-submit-release.md) · [checklist](00-checklist.md) · próxima: [07](07-dispatch-issue.md)

## Objetivo

P.O. comenta ou pede ajuste num PR, pelo MCP — nunca aprova nem faz merge
(isso continua exclusivo do humano, `/merge`). Com limite de rodadas de
"pedir ajuste" por PR, pra não entrar num loop infinito agente↔MCP.

## Arquivos tocados

- `crates/oreia-seca/src/maker/mcp.rs` — `review_pr(repo, n, verdict, comment)`.
- `crates/oreia-seca/src/config.rs` — `[mcp].max_review_rounds` (default 3).

## Decisões de design

- **`verdict = comment`**: `ForgejoClient::review_pull(..., ReviewEvent::Comment, comment)`
  direto — sem limite de rodadas (só comentário, não força retrabalho).
- **`verdict = request_changes`**: mesmo caminho de `run_changes` em
  `maker/commands.rs` (`review_pull(RequestChanges)` + achar o agente dono do
  branch via `parse_agent_branch` + `dispatch_work` se for branch de agente)
  — reaproveitar aquela função (ou extrair a parte comum numa privada
  compartilhada entre `/changes` do Telegram e `review_pr` do MCP), não
  duplicar a lógica.
- **Limite de rodadas**: contar, em `data/mcp.jsonl` (o mesmo log de
  auditoria exigido pela seção de Segurança — sem store novo), quantas
  linhas têm `tool = "review_pr"`, `result` indicando `request_changes`, e o
  mesmo `repo#n`. Na chamada que estouraria `max_review_rounds` (padrão 3):
  recusa a ferramenta (erro MCP claro) e `Maker::broadcast` um aviso nos
  canais humanos pedindo decisão.
- Esta ferramenta não verifica posse (`checkout_issue`/claim) — revisar um PR
  não exige ter assumido a issue.

## Testes obrigatórios

- `review_pr(comment)` só comenta, não dispara nada.
- `review_pr(request_changes)` aciona o agente dono do branch (regressão: PR
  de um `agent/<nome>/issue-n` real).
- A quarta rodada com `max_review_rounds = 3` é recusada e gera aviso no
  Telegram — usar o `Recorder` de teste já existente nos canais
  (`oreia-channels`, conferir o nome exato do helper de teste ao abrir o
  crate) pra confirmar a mensagem chegou.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [ ] `review_pr` funcionando nos dois vereditos, com o limite respeitado.
- [ ] Checklist atualizado com o commit.
