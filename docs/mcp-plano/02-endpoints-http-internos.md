# Etapa 2 — Endpoints HTTP internos

[← 01](01-posse-persistente.md) · [checklist](00-checklist.md) · próxima: [03](03-crate-mcp-stdio-leitura.md)

## Objetivo

O processo MCP (etapa 3+) nunca lê/escreve estado direto — ele fala com o
orquestrador JÁ RODANDO pela API HTTP interna, como qualquer outro cliente
REST. Esta etapa cria esses endpoints (a lógica de cada um chega nas etapas
seguintes; aqui entra só o esqueleto, a autenticação e os que já têm tudo que
precisam — leitura).

## Arquivos tocados

- `crates/oreia-seca/src/http.rs` — rotas novas sob `/api/mcp/*`.
- `crates/oreia-seca/src/maker/mcp.rs` (novo) — métodos de `Maker` que cada
  endpoint chama (`create_story`, `list_stories`, `get_story`, `get_pr`,
  `review_pr`, `checkout_issue`, `run_checks`, `submit_issue`,
  `release_issue`, `dispatch_issue`, `mcp_stats`). Nesta etapa, só os de
  leitura (`list_stories`, `get_story`, `get_pr`, `mcp_stats`) ganham
  implementação real; os demais ficam com assinatura + `todo!()`/erro
  "ainda não implementado", destravados nas etapas 4-7.
- `crates/oreia-seca/src/maker/mod.rs` — `mod mcp;`.

## Decisões de design

- **Auth**: mesmo Bearer `REST_API_KEY`/`authorized()` que `/api/status` e
  `/api/message` já usam — nenhum mecanismo novo nesta borda interna. O
  `MCP_TOKEN` da etapa 8 é outra borda (cliente externo ↔ processo MCP), não
  esta.
- **Uma rota POST por ferramenta**, corpo JSON, mesmo estilo de
  `/api/message`: `POST /api/mcp/{create_story,list_stories,get_story,get_pr,
  review_pr,checkout_issue,run_checks,submit_issue,release_issue,
  dispatch_issue,stats}`. Mesmo as "leituras" (`get_story`, `list_stories`...)
  vão de POST — mantém tudo consistente e evita decorar quais são GET vs POST.
- **`get_pr`**: título, autor, `--stat` do diff (reaproveita
  `gitops::changed_files`, que já roda `git diff --stat`), diff truncado
  (constante de limite de caracteres — mesmo padrão de `DIFF_MAX_CHARS` em
  `executor/verify.rs`), estado do CI (se o Forgejo expuser via
  `ForgejoClient` — senão, omitir o campo), `REVIEW.md` se existir no
  workdir, seção de escalada (igual ao que `ForgejoCtx::publish` já monta a
  partir de `task.phases`).
- **`list_stories`**: `status` = `all | available | in_progress | done`.
  "available" = issue sem label de agente (`agent_by_label` não bate) OU
  última `Task` daquele `repo#issue` com `status=Failed`/`DoneWithErrors` OU
  tem PR aberto aguardando ajuste (reaproveita a mesma lógica de
  `already_delivered`/`list_open_pulls`). Precisa iterar
  `self.forgejo.owner.unwrap().list_open_pulls` + `TaskStore::list()`
  filtrando por `work_item`.
- **`mcp_stats`**: delega direto pra `crate::executor::log::load` +
  `aggregate`, igual ao comando `/stats` em `maker/commands.rs` — sem
  duplicar a agregação.

## Testes obrigatórios

- Cada rota nova exige o Bearer (401 sem ele / com token errado), igual aos
  testes já existentes em `http.rs::tests` (`message_requires_token`,
  `check_requires_token`).
- `get_story`/`list_stories`/`get_pr`/`stats` contra um `Maker` de teste
  (`maker::tests::test_maker`) com Forgejo simulado, verificando o JSON de
  resposta.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [x] Rotas registradas, autenticação testada (inclusive os stubs 501 — nenhuma rota de
      `/api/mcp/*` fica sem o Bearer).
- [x] `list_stories`, `get_story`, `get_pr`, `stats` funcionando de ponta a ponta.
- [x] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes (140 testes).
- [x] Checklist atualizado com o commit.

Notas de implementação:
- A autenticação do Bearer precisa ser checada ANTES de decodificar o corpo JSON — o
  extractor `Json<T>` do axum roda antes do corpo do handler e devolveria 422 (corpo
  inválido) no lugar de 401 pra quem nem tem o token. Os handlers de `/api/mcp/*`
  recebem `Bytes` cru e chamam `authorized_json` (checa o Bearer, só então decodifica).
- `list_stories` precisou de dois métodos novos em `ForgejoClient`
  (`list_issues`, inexistente antes) e `get_pull_diff` (usa o endpoint `.diff` do
  Forgejo/Gitea em vez de clonar o repo só pra rodar `git diff` — mais simples e
  funciona pra QUALQUER PR aberto, não só os que o MCP já tem clonado localmente).
- `PhaseStatus` ganhou `as_str()` (faltava, ao contrário de `TaskStatus`) e `Task` ganhou
  `escalation_summary()`, reaproveitado também por `ForgejoCtx::publish` (removeu a
  duplicação que já existia ali).
