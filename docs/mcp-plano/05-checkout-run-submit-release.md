# Etapa 5 — `checkout_issue` / `run_checks` / `submit_issue` / `release_issue`

[← 04](04-create-story.md) · [checklist](00-checklist.md) · próxima: [06](06-review-pr-com-limite.md)

Etapa mais arriscada do pedido: é aqui que o MCP realmente manipula git e
abre PR. Seguir à risca a regra de segurança do plano geral — o token nunca
sai do processo `oreia-seca` (as tools do `oreia-mcp` só veem caminhos e
texto, nunca o `AGENT_MCP_TOKEN`).

## Objetivo

- `checkout_issue(repo, n)`: toma a posse, prepara o branch (continua de um
  agente que falhou, ou cria do zero), devolve tudo que o modelo do cliente
  precisa pra trabalhar.
- `run_checks(repo, n)`: roda a MESMA verificação do `executor/verify.rs` no
  diretório do checkout.
- `submit_issue(repo, n, summary)`: só com checks verdes — commit+push com
  `agent-mcp`, abre/atualiza o PR, comenta na issue.
- `release_issue(repo, n, return_to_agent?)`: libera a posse.

## Arquivos tocados

- `crates/oreia-seca/src/maker/mcp.rs` — as quatro funções.
- `crates/oreia-seca/src/config.rs` — completar `[mcp]` com `workdir` (default
  `workspace/mcp`).
- `crates/oreia-seca/src/maker/forge.rs` — pode precisar de um construtor de
  `GitAuth`/`ForgejoClient` avulso pro `agent-mcp` (ele não está em
  `ForgejoCtx::agents`, que só guarda os agentes de `[[agents]]`) — adicionar
  um campo `pub mcp: Option<McpAgent>` em `ForgejoCtx` (mesma forma de `Agent`,
  token de `AGENT_MCP_TOKEN`), construído em `ForgejoCtx::build`.

## Decisões de design

- **Diretório de trabalho**: `{[mcp].workdir}/{owner}-{repo}-{n}` (ex.
  `workspace/mcp/acme-app-42`), não `workspace/<task-id>` (que é efêmero e
  apagado a cada rodada em `exec.rs`) — este precisa sobreviver entre
  chamadas de `checkout_issue`/`run_checks`/`submit_issue` na mesma issue.
  Documentar MUITO claramente em `docs/mcp.md` (etapa 9) que o cliente MCP
  precisa enxergar esse caminho — funciona de graça quando roda na mesma
  máquina; servidor remoto é fase 2, fora de escopo (nota do pedido
  original).
- **`checkout_issue`**:
  1. `self.mcp_claim(o, r, n)` já ocupado → recusa, devolve quem (dono atual:
     nome do agente ou "mcp", e a fase/status da `Task` dele, lido de
     `TaskStore::list()` filtrando por `work_key`).
  2. Livre → `claim_mcp` + label `claimed:mcp` (via `ForgejoClient::add_issue_labels`)
     + comentário "🤚 Assumida via MCP..." (`comment_issue` com o cliente do
     `agent-mcp`).
  3. Branch: olhar se existe `agent/<algum-agente>/issue-{n}` no remoto
     (reaproveitar `gitops` — talvez precise de uma nova função pequena
     `gitops::remote_branch_exists`, já que a lógica equivalente hoje está
     inline em `gitops::prepare`); se existir, `gitops::prepare` com esse
     branch (continua o trabalho do agente); senão, branch nova
     `agent/mcp/issue-{n}`. Em ambos os casos, reaproveitar `gitops::prepare`
     tal como está — ele já sabe continuar de um branch remoto existente.
  4. `FailureContext`: buscar a última `Task` com esse `work_key` e
     `status = Failed`/`DoneWithErrors`; se achar, montar um resumo (fases
     ok/falhas, `phase.summary`, `test_output`/`diff` se o `Phase` guardar
     isso — conferir se `Phase` precisa de um campo novo pra isso, já que
     hoje `FailureContext` só vive dentro da execução de uma fase, não é
     persistido em `Task`/`Phase`. **Decisão a confirmar ao abrir
     `maker/exec.rs`/`oreia-core/task.rs` nesta etapa**: provavelmente
     adicionar `pub failure: Option<FailureContextSummary>` em `Phase`,
     preenchido quando a fase falha de vez (todos os degraus esgotados),
     serializável — hoje só `summary: Option<String>` existe.
  5. Devolve: caminho do workdir, branch, texto da issue
     (`ForgejoClient::get_issue`), critérios de aceite (parseados do corpo —
     mesma seção "## Critérios de aceite" que `create_story` escreve, se
     presente) e o `FailureContext` serializado, se houver.
- **`run_checks`**: reaproveita `crate::executor::verify::verify` direto —
  mesma assinatura (`workdir`, instrução/objetivo da issue como `instruction`,
  resultado sintético `Ok(PhaseOutcome { changed_files: executor::git_changed_files(workdir).await, ..Default::default() })`,
  `test_cmd` de `[escalation.<papel-do-agente-que-tentou>].test_cmd` ou, sem
  agente anterior, de um `[mcp].test_cmd` por repositório — **decidir ao
  abrir esta etapa**: provavelmente mais simples ter
  `[mcp].test_cmd_by_repo: BTreeMap<String, String>` ou, mais simples ainda,
  reaproveitar o `test_cmd` do papel do `agent` mais recente que tocou a
  issue (se houver) e, sem nenhum, sem `test_cmd` (modo fraco, como já existe
  hoje) — não inventar config nova se o existente já resolve.
- **`submit_issue`**: `verify()` primeiro — reprovado, devolve o motivo e a
  saída truncada, SEM commitar nada. Aprovado:
  - `gitops::commit_and_push(dir, &branch, message, &mcp_auth)` (mesma
    função de sempre, `GitAuth::new("agent-mcp", token)`).
  - **Nunca em `main`**: antes de tudo, `anyhow::ensure!(branch != base_branch, ...)`
    — o branch atual (de `checkout_issue`) é sempre `agent/mcp/issue-{n}` ou
    o branch de um agente (`agent/<nome>/issue-{n}`), nunca a base, então
    isso é defesa em profundidade, não o caminho normal.
  - PR novo (`create_pull`) ou atualiza o existente do mesmo branch (mesmo
    tratamento de 409/422 que `ForgejoCtx::publish` já faz — reaproveitar
    aquele bloco, extraído pra uma função compartilhada se fizer sentido).
  - Comenta na issue com o `summary` recebido.
  - Mantém a posse (`claim_mcp` continua lá) até merge ou `release_issue`.
- **`release_issue(return_to_agent)`**: `release_mcp` sempre remove a claim e
  a label `claimed:mcp`; com `return_to_agent = true`, reaplica a label do
  agente original (de qual? — guardar o nome do agente dono do branch
  original, se `checkout_issue` partiu de um branch de agente; sem isso,
  recusa `return_to_agent` com uma mensagem clara, já que não há "agente
  original" quando a issue nunca teve dono).

## Testes obrigatórios

- `checkout_issue` recusa quando outro dono está ativo (diz quem e em qual
  fase) — testado com Forgejo simulado + um `Maker` com uma claim de agente
  já presente.
- `checkout_issue` aceita issue livre.
- `checkout_issue` aceita issue com `Task` `Failed`, devolve `FailureContext`,
  parte do branch do agente (git real + Forgejo simulado, como
  `gitops.rs::tests::prepare_commit_push_local_remote` e `maker/tests.rs`).
- `run_checks` com `test_cmd` falhando → resultado vermelho.
- `submit_issue` recusa quando `run_checks` reprovaria.
- `submit_issue` com checks verdes: commit com autor `agent-mcp`, PR criado;
  segunda chamada atualiza o mesmo PR (não cria um segundo).
- `release_issue(return_to_agent=true)` reaplica a label e o webhook volta a
  disparar o agente (regressão ponta a ponta).
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes.

## Critério de pronto

- [x] As quatro ferramentas funcionando de ponta a ponta contra Forgejo simulado + git
      real (issue livre, issue com agente que falhou — continua o branch dele e reporta
      o motivo —, checks vermelhos recusando `submit_issue`, PR criado e atualizado na
      segunda chamada, `release_issue` reaplicando a label e o webhook disparando o
      agente de novo).
- [x] Nunca faz push em `main`/branch base (`submit_issue` recusa antes de tentar).
- [x] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` verdes (150 testes).
- [x] Checklist atualizado com o commit.

Decisões tomadas ao abrir o código (conforme a nota "a confirmar" deixava em aberto):
- **Sem campo novo em `Phase`/`Task` pra `FailureContext`.** `checkout_issue` monta o
  resumo da falha a partir do que `Phase` já guarda (`name`, `status`, `summary`,
  `resolved_step`) — suficiente pro cliente MCP entender o que foi tentado, sem precisar
  persistir a saída crua de teste/diff da tentativa (isso só existe durante a execução
  da fase, não depois).
- **`test_cmd` de `run_checks`** vem do papel (`model_role`) do último `Task` registrado
  pra aquela issue, via `[escalation.<papel>].test_cmd` — sem `Task` anterior (issue
  nunca tentada por um agente), roda em modo fraco, igual à escalada sem `test_cmd`.
- **Qual branch continuar** é decidido só com chamadas já existentes do
  `ForgejoClient` (`list_open_pulls`, `list_branches`) — nada de `git ls-remote` novo.
  Ordem: PR aberto da issue vence; senão, branch do agente da label (se existir);
  senão, `agent/mcp/issue-<n>` do zero. `gitops::prepare` (já existente) cuida do resto
  (clona e continua, ou cria, sem mudança nenhuma nele).
- **`ForgejoCtx` ganhou `McpAgent`** (conta própria, igual a `Agent` mas sem label/user
  de `[[agents]]`) e `prepare_repo_for_mcp`/`comment_as_mcp`/`publish_mcp` — espelham
  `prepare_repo`/`comment_as_agent`/`publish`, mas nunca tocam na posse/label de agente.
