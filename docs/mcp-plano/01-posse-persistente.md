# Etapa 1 — Posse persistente + conta `agent-mcp`

[← checklist](00-checklist.md) · próxima: [02](02-endpoints-http-internos.md)

## Objetivo

Trocar o `Mutex<HashSet<String>>` em memória (`Maker::active_work`) por posse
persistente em `data/claims.json`, e criar a conta `agent-mcp` no Forgejo (via
`bootstrap.sh`) para ser usada pelas etapas seguintes. Sem isso, nenhuma das
etapas seguintes tem onde registrar "esta issue está com o MCP".

## Arquivos tocados

- `crates/oreia-seca/src/maker/claims.rs` (novo) — `ClaimStore`, `Claim`,
  `Owner` (`Agent(String)` | `Mcp`).
- `crates/oreia-seca/src/maker/mod.rs` — `active_work: Mutex<HashSet<String>>`
  vira `claims: ClaimStore`; `claim_work`/`release_work`/`is_active` passam a
  delegar pra ele (mesmas assinaturas — nenhum chamador muda). Reconciliação
  de boot em `Maker::new`.
- `crates/oreia-seca/src/maker/forge.rs` — `handle_forgejo`: corte por posse
  do MCP logo no início de `Issues`/`IssueComment`.
- `crates/oreia-seca/src/config.rs` — seção `[mcp]` (só os campos que esta
  etapa já usa: `enabled`, `agent_user`, `agent_token_env`; o resto
  — `workdir`, `max_review_rounds`, `http`, `token_env`, `bind` — entra nas
  etapas que precisarem). `is_agent_user` passa a considerar `cfg.mcp`.
- `scripts/bootstrap.sh` — seção nova (fora do loop de `[[agents]]`) criando
  o usuário `agent-mcp`, o token (`AGENT_MCP_TOKEN`) e, no loop por
  repositório, as labels `agent:mcp` e `claimed:mcp`.
- `oreia-seca.example.toml` — `[mcp]` comentado.
- `.env.example` — `AGENT_MCP_TOKEN=`.

## Decisões de design

- **Schema de `data/claims.json`**: mapa `"owner/repo#issue" → Claim`, onde
  `Claim { owner: Owner, since: DateTime<Utc>, task_id: Option<String> }`.
  Escrita atômica (tmp + rename), mesmo padrão de
  `oreia-core::store::TaskStore` (`crates/oreia-core/src/store.rs`).
- **API pública mantida**: `claim_work(&WorkItem) -> bool`,
  `release_work(&WorkItem)`, `is_active(&WorkItem) -> bool` continuam
  existindo com a mesma assinatura (usadas por `forge.rs`/`exec.rs` sem
  mudança) — por baixo, batem no `ClaimStore` com `Owner::Agent(w.agent.clone())`.
- **API nova pro MCP**: `claim_mcp(repo, issue, task_id) -> bool`,
  `release_mcp(repo, issue, return_to_agent: bool)`,
  `mcp_claim(repo, issue) -> Option<Claim>`. `release_mcp` com
  `return_to_agent = true` não mexe no `ClaimStore` sozinho — quem chama
  (etapa 5, `release_issue`) decide se reaplica a label do agente antes/depois.
- **Reconciliação no boot** (`Maker::new`, depois de `TaskStore::open`): para
  cada `Claim { owner: Owner::Agent(_), .. }` sem `Task` com `status=Running`
  e `task_id` igual, remove do `ClaimStore` e loga em `tracing::warn`. Claims
  `Owner::Mcp` nunca são tocadas no boot (persistem entre reinícios do
  processo, é o objetivo da etapa).
- **Corte no `handle_forgejo`**: antes de qualquer outra checagem em
  `Event::Issues(p)` e `Event::IssueComment(p)`, se
  `self.mcp_claim(&owner, &repo, issue).is_some()` OU a issue/PR tem a label
  `claimed:mcp`/`agent:mcp` no payload, `return` sem despachar. Cobre
  `opened`, `reopened`, `label_updated`, `labeled` e comentário.
- **`agent-mcp` não entra em `[[agents]]`** — ficaria sujeito a
  `agent_by_label` e disparia a via automática. É uma conta separada, só
  usada para `git commit`/push/PR autorados (`GitAuth` com o token
  `AGENT_MCP_TOKEN`, reaproveitando `crates/oreia-seca/src/gitops.rs` sem
  mudança nele).

## Execução ao vivo (autorizada pelo usuário)

Depois de editar `bootstrap.sh`, rodar `make bootstrap` (ou os comandos
`docker compose exec -T -u git forgejo forgejo admin user create/generate-access-token`
equivalentes) contra o stack que já está no ar, e confirmar:

```
docker compose exec -T -u git forgejo forgejo admin user list   # agent-mcp aparece
grep AGENT_MCP_TOKEN .env                                        # token presente
```

## Testes obrigatórios

- Posse persistente sobrevive a recriar o `Maker` (abre um `ClaimStore` do
  mesmo `data/claims.json`, simulando restart do processo).
- Webhook (`Event::Issues`/`IssueComment`) com a issue carregando
  `claimed:mcp` é ignorado por `handle_forgejo` (nenhum `dispatch_work`
  chamado) — reaproveita o Forgejo simulado de `maker/tests.rs`.
- Reconciliação de boot: uma claim de agente sem `Task` `Running`
  correspondente é liberada; uma claim `Mcp` sobrevive ao boot.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes
  (todos os testes já existentes continuam passando — `active_work` some,
  mas o comportamento observável de `claim_work`/`is_active` não muda).

## Critério de pronto

- [x] Código compila e testes verdes (139 testes, incluindo os 6 novos desta etapa).
- [x] `cargo fmt --check` e `clippy -D warnings` limpos.
- [x] `agent-mcp` criado de verdade no Forgejo ativo (`docker compose exec forgejo admin user list` confirma), token em `AGENT_MCP_TOKEN` no `.env`; labels `agent:mcp`/`claimed:mcp` criadas em `minha-org/meu-sistema`.
- [x] Checklist atualizado com o commit.

Nota: `claim_mcp`/`release_mcp` ficaram com `#[allow(dead_code)]` nesta etapa — sem
chamador de produção até a etapa 2 ligar `checkout_issue`/`release_issue` aos
endpoints HTTP. Remover o `allow` quando isso acontecer.
