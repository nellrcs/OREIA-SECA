# Etapa 3 — Crate `oreia-mcp`: stdio + ferramentas de leitura

[← 02](02-endpoints-http-internos.md) · [checklist](00-checklist.md) · próxima: [04](04-create-story.md)

## Objetivo

Criar o crate de biblioteca `oreia-mcp` com o servidor MCP de verdade
(`rmcp`), expondo pelo transporte stdio as ferramentas de leitura já prontas
na etapa 2 (`list_stories`, `get_story`, `get_pr`, `get_stats`) + os recursos
e o prompt. É aqui que `tools/list` passa a responder de verdade.

## Antes de escrever o primeiro `#[tool]`

Confirmar a sintaxe exata da versão fixada (`rmcp = "3.8"`) lendo os exemplos
do próprio crate — rodar (dentro do container de build, que já terá o
registry baixado):
```
docker run --rm -v "<repo>:/src" -w /src rust:1-bookworm \
  bash -c "cargo doc -p oreia-mcp --no-deps 2>&1 | tail -30; find / -path '*/rmcp-3.8.0/examples*' -name '*.rs' 2>/dev/null | head"
```
ou consultar `https://github.com/modelcontextprotocol/rust-sdk` (pasta
`examples/servers`) via `WebFetch`. Não adivinhar a macro.

## Arquivos tocados

- `Cargo.toml` (raiz) — novo membro `crates/oreia-mcp`.
- `crates/oreia-mcp/Cargo.toml` — `rmcp = { version = "3.8", features = ["server", "macros", "transport-io", "schemars"] }`,
  `reqwest` (cliente do `/api/mcp/*`), `serde`/`serde_json`, `tokio`, `schemars`.
- `crates/oreia-mcp/src/lib.rs` — `OreiaMcp` (struct com `base_url: String`,
  `api_key: Option<String>`, `http: reqwest::Client`), `impl ServerHandler`.
- `crates/oreia-mcp/src/tools.rs` — `#[tool_router]`/`#[tool]` para
  `list_stories`, `get_story`, `get_pr`, `get_stats`, `check_config` (só
  registrada se o endpoint correspondente existir — ver nota abaixo).
- `crates/oreia-mcp/src/resources.rs` — `oreia://agents/<nome>`,
  `oreia://repos/<repo>/contract`, `oreia://docs/roadmap`.
- `crates/oreia-mcp/src/prompts.rs` — `po_guidelines` (texto estático).
- `crates/oreia-seca/src/main.rs` — subcomando `Some("mcp") => { ... }` (igual
  ao padrão de `"setup"`/`"status"` já existente), lendo `[mcp]` do config
  carregado e subindo `oreia_mcp::serve_stdio(base_url, api_key)`.

## Decisões de design

- Cada tool do `oreia-mcp` é um cliente HTTP fino: monta o JSON, `POST
  {base_url}/api/mcp/{tool}`, devolve a resposta (ou erro MCP). Nenhuma
  lógica de negócio mora no crate `oreia-mcp` — tudo fica em
  `maker::mcp` (etapa 2), testável sem subir o protocolo MCP.
- `check_config`: só registrar a tool se `maker::mcp` expuser um endpoint
  `check_config` de verdade (a feature `crate::check::run` já existe e é
  completa — então registrar). Confirmar isso lendo `crates/oreia-seca/src/check/mod.rs`
  antes de decidir, como o pedido original pede.
- Recurso `oreia://agents/<nome>`: a lógica de ler o arquivo é só
  `maker::mcp` lendo `self.cfg.agent(nome)?.prompt` (mesmo campo que
  `ForgejoCtx::agent_prompt` usa) — exposto via um endpoint
  `GET /api/mcp/resources/agents/{nome}` novo em `http.rs`, ou embutido como
  parte de `list_stories`/outra chamada existente; decidir pelo que for mais
  simples ao abrir o código (provavelmente um endpoint dedicado, para não
  acoplar recursos MCP a tools MCP).
- Prompt MCP é estático — não precisa de endpoint HTTP, fica só no crate
  `oreia-mcp`.

## Testes obrigatórios

- **Contrato MCP**: teste de integração com o transporte stdio de verdade
  (subir `oreia-seca mcp` como subprocesso de teste, ou usar o client
  embutido do `rmcp` contra o `ServerHandler` em processo) verificando que
  `tools/list` devolve as ferramentas esperadas com schemas válidos (item 9
  da lista de testes do pedido original).
- `list_stories`/`get_story`/`get_pr`/`get_stats` chamando de ponta a ponta
  contra um `oreia-seca` de teste rodando (HTTP real nesta camada, não
  mockado) — ou, se mais simples, contra um servidor HTTP falso que devolve
  JSON fixo, confirmando que o `oreia-mcp` monta a chamada certa e traduz a
  resposta certa.
- **Nenhuma resposta de ferramenta expõe token/chave** (teste com valor
  sentinela no `api_key`/`AGENT_MCP_TOKEN` de teste, conferindo que não
  aparece em nenhum campo de retorno das tools nem em mensagens de erro).
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [ ] `oreia-seca mcp` sobe e responde `tools/list`/`resources/list`/`prompts/list` via stdio.
- [ ] As 4-5 ferramentas de leitura funcionam de ponta a ponta.
- [ ] Nenhum vazamento de segredo (teste sentinela verde).
- [ ] Checklist atualizado com o commit.
