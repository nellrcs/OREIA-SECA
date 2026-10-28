# Etapa 4 — `create_story`

[← 03](03-crate-mcp-stdio-leitura.md) · [checklist](00-checklist.md) · próxima: [05](05-checkout-run-submit-release.md)

## Objetivo

Primeira ferramenta de ESCRITA: cria a issue com um template fixo. É a mais
simples das ferramentas de escrita (reaproveita quase tudo de
`create_issue_and_reply` em `maker/commands.rs`), boa pra validar o padrão
"endpoint HTTP → `maker::mcp` → `ForgejoClient`" antes das ferramentas mais
complexas da etapa 5.

## Arquivos tocados

- `crates/oreia-seca/src/maker/mcp.rs` — `create_story(repo, title, goal,
  acceptance_criteria: Vec<String>, likely_files: Vec<String>, out_of_scope:
  Option<String>, agent: Option<String>) -> anyhow::Result<CreateStoryResult>`.
- `crates/oreia-seca/src/http.rs` — `POST /api/mcp/create_story` já roteado
  na etapa 2, agora com corpo de verdade.
- `crates/oreia-mcp/src/tools.rs` — tool `create_story`.
- `crates/oreia-seca/src/maker/mcp.rs` (ou um `audit.rs` companheiro) —
  primeira escrita em `data/mcp.jsonl` (ver seção de Segurança no plano
  geral): `{tool, repo, issue, result, at}`, sem conteúdo sensível.

## Decisões de design

- **Template fixo da issue** (corpo em Markdown):
  ```
  ## Objetivo
  {goal}

  ## Critérios de aceite
  - [ ] {criterio 1}
  - [ ] {criterio 2}
  ...

  ## Arquivos prováveis
  - {likely_files[0]}
  ...

  ## Fora de escopo
  {out_of_scope ou "(nenhum)"}
  ```
- **Recusa sem critério de aceite**: `anyhow::ensure!(!acceptance_criteria.is_empty(), ...)`
  antes de qualquer chamada de rede — erro MCP claro, não um 500 genérico.
- **`agent` opcional**: presente → resolve a label do agente (mesmo código de
  `create_issue_and_reply`: `owner.list_labels` + achar pelo `AgentConfig.label`)
  e cria a issue já com ela, disparando a via automática normalmente (esta
  ferramenta não precisa reimplementar o dispatch — o webhook de "issue
  aberta com label" já cuida disso, igual a hoje). Ausente → issue livre, sem
  label, sem posse de ninguém.
- **Reaproveitar, não duplicar**: extrair a resolução de label por nome de
  agente de `create_issue_and_reply` (`maker/commands.rs`) para um helper
  privado compartilhado (ex. `Maker::resolve_agent_label(o, r, agent_name)`),
  chamado tanto pelo `/issue` do Telegram quanto por `create_story` — não
  reescrever a lógica duas vezes.

## Testes obrigatórios

- `create_story` sem critérios de aceite → erro, nenhuma chamada HTTP ao
  Forgejo simulado.
- `create_story` com `agent` → issue criada com a label certa (Forgejo
  simulado de `maker/tests.rs`), e o webhook de "issue aberta com label"
  dispara a via automática como hoje (teste de regressão — a via antiga não
  pode quebrar).
- `create_story` sem `agent` → issue criada sem label nenhuma.
- Entrada em `data/mcp.jsonl` registrada, sem nenhum segredo.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [ ] `create_story` funcionando de ponta a ponta (stdio → HTTP → Forgejo).
- [ ] Checklist atualizado com o commit.
