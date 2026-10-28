# Checklist — servidor MCP (P.O. + desenvolvedor sob demanda)

Acompanha a implementação de ponta a ponta. Cada etapa tem seu próprio
arquivo com objetivo, arquivos tocados, decisões de design e critério de
pronto. Marcar `[x]` e anotar o commit só depois que `cargo fmt --check`,
`clippy -D warnings` e `cargo test --workspace` passarem para aquela etapa
(rodados via o container `rust:1-bookworm`, sem toolchain Rust local nesta
máquina — ver qualquer um dos arquivos de etapa para o comando completo).

Plano completo (contexto, decisões de design já tomadas, critério de pronto
da feature inteira): `C:\Users\WRN54\.claude\plans\vast-tumbling-glacier.md`.

| # | Etapa | Arquivo | Status | Commit |
|---|---|---|---|---|
| 1 | Posse persistente (`claims.json`) + conta `agent-mcp` no Forgejo | [01](01-posse-persistente.md) | ☑ | `2210c47` |
| 2 | Endpoints HTTP internos (`/api/mcp/*`) | [02](02-endpoints-http-internos.md) | ☑ | `6ce3529` |
| 3 | Crate `oreia-mcp`: stdio + ferramentas de leitura | [03](03-crate-mcp-stdio-leitura.md) | ☑ | `c095d9b` |
| 4 | `create_story` | [04](04-create-story.md) | ☑ | `5f5c935` |
| 5 | `checkout_issue` / `run_checks` / `submit_issue` / `release_issue` | [05](05-checkout-run-submit-release.md) | ☑ | `4e13b7d` |
| 6 | `review_pr` com limite de rodadas | [06](06-review-pr-com-limite.md) | ☑ | próximo commit |
| 7 | `dispatch_issue` | [07](07-dispatch-issue.md) | ☐ | — |
| 8 | Transporte HTTP opcional do MCP | [08](08-http-opcional.md) | ☐ | — |
| 9 | `/stats` via MCP + documentação | [09](09-stats-e-docs.md) | ☐ | — |

## Critério de pronto da feature inteira

- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.
- [ ] Sem `[mcp]` no config e sem rodar `oreia-seca mcp`: comportamento idêntico ao atual.
- [ ] `oreia-seca.example.toml` com `[mcp]` comentado; `.env.example` com `AGENT_MCP_TOKEN` e `MCP_TOKEN`.
- [ ] `docs/mcp.md` + `README.md`, `docs/roadmap.md`, `docs/seguranca.md` atualizados.
- [ ] Nenhuma resposta de ferramenta/endpoint novo contém token ou segredo (teste com valor sentinela).
