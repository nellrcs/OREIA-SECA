# Etapa 9 — `/stats` via MCP + documentação

[← 08](08-http-opcional.md) · [checklist](00-checklist.md)

## Objetivo

Fechar a feature: registrar issues resolvidas via MCP em
`data/escalation.jsonl` (pra aparecer em `/stats`), e escrever toda a
documentação prometida no critério de pronto geral.

## Arquivos tocados

- `crates/oreia-seca/src/executor/log.rs` — `EscalationLogEntry` ganha
  `executor = "mcp"` como valor válido (já é uma `String`, não um enum — só
  gravar esse valor em `submit_issue`) e um campo novo
  `#[serde(default)] pub handoff_from: Option<String>` (nome do agente que
  falhou antes, se houver).
- `crates/oreia-seca/src/maker/mcp.rs::submit_issue` — grava uma entrada em
  `data/escalation.jsonl` com `executor = "mcp"` e `handoff_from` preenchido
  quando `checkout_issue` tinha achado uma `Task` `Failed` anterior.
- `crates/oreia-seca/src/maker/commands.rs` (comando `/stats`) e
  `crates/oreia-seca/src/executor/log.rs::aggregate`/`RoleStats` — linha nova
  "resolvidas via MCP após falha: N (dev-back: X, dev-front: Y)", agrupando
  por `handoff_from`.
- `docs/mcp.md` (novo).
- `README.md`, `docs/roadmap.md`, `docs/seguranca.md` — atualizados.
- `oreia-seca.example.toml` — `[mcp]` comentado e completo (todos os campos
  das etapas 1-8).
- `.env.example` — `AGENT_MCP_TOKEN`, `MCP_TOKEN`.

## `docs/mcp.md` — conteúdo mínimo

- As duas vias (automática por label vs. MCP explícito) e a regra de posse
  exclusiva.
- Como registrar no Claude Code: `claude mcp add oreia-seca -- oreia-seca mcp --config /caminho/oreia-seca.toml`
  (ou equivalente — conferir a sintaxe real de `claude mcp add` pro transporte
  stdio ao escrever este doc).
- Como registrar no Claude Desktop: trecho de `claude_desktop_config.json`
  (`mcpServers.oreia-seca.command`/`args`).
- Um fluxo de exemplo ponta a ponta: "crie a história → agente falhou →
  assumi via MCP → submit", com os nomes reais das ferramentas em cada passo.
- Nota clara sobre o diretório de trabalho (`[mcp].workdir`) precisar ser
  visível pro cliente, e o que fazer quando o `oreia-seca` roda num servidor
  remoto (documentar a alternativa descrita no pedido original — montar o
  volume, ou clonar localmente e usar `submit_issue` com push do branch
  local, citando que isso é fase 2/fora de escopo desta implementação).

## Testes obrigatórios

- `aggregate`/`RoleStats` com entradas `executor = "mcp"` e `handoff_from`
  preenchido agregam certo (teste unitário em `executor/log.rs`, no mesmo
  estilo dos já existentes).
- `/stats` (comando e `GET /api/mcp/stats`) mostra a linha nova quando há
  dados.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes
  — TODA a suíte, incluindo tudo das etapas 1-8.

## Critério de pronto (replica o critério de pronto da feature inteira)

- [x] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes (245
      testes no total: 160 em `oreia-seca`, 5 em `oreia-mcp`, 80 nos demais crates).
- [x] Sem `[mcp]` no config e sem rodar `oreia-seca mcp`: comportamento idêntico ao atual.
- [x] `oreia-seca.example.toml` com `[mcp]` comentado; `.env.example` com `AGENT_MCP_TOKEN` e `MCP_TOKEN`.
- [x] `docs/mcp.md` (novo) + `README.md`, `docs/roadmap.md`, `docs/seguranca.md`,
      `docs/configuracao.md` atualizados.
- [x] Nenhuma resposta de ferramenta/endpoint novo contém token ou segredo.
- [x] Checklist (00) com as 9 etapas marcadas e commits linkados.

Implementação: `EscalationLogEntry` ganhou `handoff_from: Option<String>` (aditivo) e
`RoleStats` ganhou `handoff_by_agent: BTreeMap<String, usize>`. `submit_issue` registra
uma entrada `role = "mcp"` em `data/escalation.jsonl` só quando publica de verdade
(`pushed`), com `handoff_from` = agente da última `Task` `Failed`/`DoneWithErrors`
daquela issue, se houver. `/stats` (comando) ganhou a linha "resolvidas via MCP após
falha: N (agente: X, ...)" quando essa seção não está vazia.
