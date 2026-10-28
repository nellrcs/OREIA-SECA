# Etapa 8 — Transporte HTTP opcional do MCP

[← 07](07-dispatch-issue.md) · [checklist](00-checklist.md) · próxima: [09](09-stats-e-docs.md)

## Objetivo

Permitir que um cliente MCP se conecte por HTTP streamable (em vez de stdio
local), para quando o cliente não roda na mesma máquina que o `oreia-seca mcp`.
Opt-in, só com token obrigatório.

## Arquivos tocados

- `crates/oreia-mcp/Cargo.toml` — feature `transport-streamable-http-server`.
- `crates/oreia-mcp/src/http_transport.rs` (novo) — `StreamableHttpService`
  do `rmcp`, com checagem de `Origin` e do Bearer `MCP_TOKEN` antes de
  delegar ao serviço.
- `crates/oreia-seca/src/config.rs` — completar `[mcp]`: `http: bool`
  (default `false`), `bind` (default `127.0.0.1:3121` ou similar), `token_env`
  (default `MCP_TOKEN`).
- `crates/oreia-seca/src/main.rs` — subcomando `mcp`: se `[mcp].http = true`,
  sobe o transporte HTTP além do stdio (ou no lugar dele — decidir ao
  implementar: provavelmente os dois podem coexistir, stdio pra uso local e
  HTTP pra remoto).

## Decisões de design

- **Sem `MCP_TOKEN` configurado → HTTP não sobe** (recusa já no boot do
  subcomando `mcp`, com mensagem clara — mesmo padrão de
  `ForgejoCtx::build` avisando sobre token ausente).
- **Bind em `127.0.0.1` por padrão** — expor em outra interface é escolha
  explícita do usuário (`[mcp].bind`), documentada com o aviso de que isso
  expõe a ferramenta de escrita (`submit_issue` etc.) pra quem alcançar a
  porta.
- **Validação de `Origin`**: allowlist simples (ou exigir ausência de
  `Origin` — requisição de servidor pra servidor — OU um valor esperado
  configurável); `Origin` presente e fora da allowlist → 403. Olhar como o
  `rmcp` already expõe isso no `StreamableHttpService` (pode já ter um hook)
  antes de reimplementar a checagem na mão.

## Testes obrigatórios

- Sem `MCP_TOKEN` → o subcomando `mcp --http` recusa subir (processo sai com
  erro, não "sobe mas sem auth").
- `Origin` inválido → 403.
- Bearer `MCP_TOKEN` correto → `tools/list` funciona sobre HTTP igual ao
  stdio.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [ ] Transporte HTTP funcionando, opt-in, com as duas checagens de segurança.
- [ ] Checklist atualizado com o commit.
