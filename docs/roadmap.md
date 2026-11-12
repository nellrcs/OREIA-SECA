# Roadmap

## Pronto (v0.1)

- [x] Orquestrador em Rust com fila, aprovação cross-canal, classificador e sessões
- [x] Roteador de modelos por papel com reserva, timeout e cooldown
- [x] Qualquer modelo OpenAI-compatível (Ollama, LM Studio, llama.cpp, vLLM, OpenRouter, Gemini)
- [x] Telegram com botões de aprovação e whitelist; terminal; API REST
- [x] Webhook do Forgejo com HMAC; agentes por label; `@menção` para ajustes
- [x] Clone → fases → commit/push → PR → comentário na issue, com a conta do agente
- [x] `/prs`, `/merge`, `/changes` pelo Telegram
- [x] Skills em Markdown + shell; ações confinadas ao diretório da tarefa
- [x] Bootstrap do Forgejo (usuários, tokens, repos, labels, webhook, proteção do `main`)
- [x] 72 testes, incluindo o fluxo issue → PR com modelo falso, Forgejo simulado e git real

## Pronto (v0.2)

- [x] Executor de fases plugável (`builtin` + CLIs externas: Aider, OpenCode, Goose, Codex)
- [x] Escada de escalada por papel: degraus com verificação por `test_cmd`, aprovação a partir de um degrau e teto de custo — veja [Executores e escalada](executores.md)
- [x] `data/escalation.jsonl` e `/stats [papel]`

## Pronto (v0.3)

- [x] LLM como serviço externo configurável: `docker-compose.yml` nunca sobe modelo junto — sempre um servidor externo (outra máquina, LM Studio, OpenRouter...)
- [x] `base_url_env`/`model_env` em `[models.*]`, com boot tolerante a modelo não configurado ou inacessível
- [x] `oreia-seca setup llm` (`make llm-setup`): assistente interativo ou por flags que lista modelos do servidor e grava o `.env`
- [x] `/models` e `GET /api/status` mostram o estado de cada modelo (`não configurado` | `inacessível` | `ok`), sem vazar chaves

## Pronto (v0.4)

- [x] `oreia-seca setup` unificado: Telegram, LLM e bootstrap do Forgejo numa coisa só, de três formas — veja [Instalação](instalacao.md)
- [x] Wizard de terminal (`make setup`), página web local sem autenticação (`make setup-web`) e arquivo de respostas/flags não interativo (`make setup-answers`), todos chamando a mesma `apply_answers`
- [x] `setup.answers.example.toml`: o formato pensado para automatizar ou para um agente de IA preencher e aplicar sozinho

## Pronto (v0.5)

- [x] `./oreia <comando>` e `oreia-seca status|up|down`: tudo que o Makefile faz, sem precisar de `make` — validado contra um `make` de fato ausente num teste real
- [x] Portas configuráveis (`FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT` no `.env`) e checagem antes de subir, com erro claro em vez do erro cru do Docker quando a porta já está em uso
- [x] `make status`/`./oreia status`: containers + health check do orquestrador e do Forgejo numa coisa só
- [x] Telegram repete o envio de mensagem (3 tentativas, com espera) em falha de transporte, em vez de simplesmente perder a resposta numa rede instável
- [x] Corrigido falso positivo na checagem "nenhum arquivo mudou, mas a fase pedia mudança de código": usava o texto inteiro da instrução (onde um verbo de mudança podia aparecer por outro motivo, ex. "do NOT create files"), agora usa só o nome da fase
- [x] Segundo provedor de modelo, `anthropic` (API nativa da Anthropic, `/v1/messages`, com `AnthropicNative` em `oreia-llm`), para usar Claude com uma chave direto do console.anthropic.com — veja [Modelos locais](modelos-locais.md#anthropic). Já configurado como segundo degrau de `[escalation.dev-back]`/`[escalation.dev-front]` no `oreia-seca.toml`
- [x] Planner não força mais fase de build/teste em toda tarefa — só quando o projeto já tem suíte e a mudança é arriscada o bastante pra justificar; o executor também ganhou a regra explícita de não rodar build/lint/teste por conta própria fora disso
- [x] Revisão automática: `WorkItem`/`Task` ganharam um `mode` (`Code`/`Review`); ao abrir um PR, se `reviewer` estiver configurado, ele entra sozinho na mesma branch (não cria uma nova), roda build/lint/teste e **comenta** no PR — nunca aprova, reprova ou faz merge (isso continua só via `/merge`/`/changes`, com o seu token). Veja [Agentes](agentes.md#o-revisor)
- [x] `deepseek` via OpenRouter respondendo em `tool_calls` nativo (`content: null`) em vez do texto `<action>` — tratado em `oreia-llm` (traduz de volta pro formato `<action>`, OpenAiCompat genérico, qualquer provedor se beneficia)
- [x] `expects_code_change`: lista de verbos ampliada (apply, change, set, replace, adjust, style, rewrite, move, copy, insert, append, extend) — fases como "Apply red background to index" passavam a checagem sem mudar nenhum arquivo
- [x] Executor externo: `message_file` virou caminho absoluto (estava relativo a `req.workdir`, que também é o `cwd` do comando — dava caminho duplicado e "No such file" nos presets que usam `< {message_file}`); prompt enviado à CLI externa ganhou contexto de repositório e regras, igual ao builtin
- [x] Os 5 executores (builtin + aider/opencode/goose/codex) testados de verdade contra a API da OpenRouter (deepseek e google/gemini-2.5-flash-lite), inclusive o builtin de ponta a ponta (não só o provedor HTTP) — 3 das 4 flags `--api-base`/`--api-key` não existiam nas ferramentas reais; corrigido um por um com o mecanismo que cada CLI de fato usa. O opencode também validava o modelo contra o catálogo próprio dele, rejeitando um modelo que a API serve de verdade — corrigido com um provedor `oreiaseca` ad-hoc (`@ai-sdk/openai-compatible`) que não depende desse catálogo. Veja [Executores e escalada](executores.md) e `scripts/live_test_setup.sh`
- [x] Vazamento de token interno do deepseek (`<｜DSML｜ ...｜>`) no lugar de `<param>`/`</param>`/`</action>` — visto de novo em produção numa variante mais grave (tag de ABERTURA também vazada, duas chamadas de `shell` na mesma resposta). `normalize_model_markup` ampliado pra cobrir abertura E fechamento, com os 5 textos reais capturados como teste
- [x] `expects_code_change`: falso positivo real ("Verify the change", fase só de checagem, falhava porque "change" aparece como substantivo no meio da frase) — a checagem agora olha só a primeira palavra da fase (o verbo que abre, como o planner sempre escreve), não a frase inteira por substring
- [x] Branch base por issue: o campo "Ramo" do Forgejo ao abrir a issue (`issue.ref`) agora é respeitado — clona, cria a branch do agente e abre o PR contra essa branch em vez de sempre `[forgejo].base_branch`, útil pra trabalhar em cima de uma feature branch já existente. O `reviewer` também passou a comparar o diff contra a base REAL do PR (`pull_request.base`), não contra um palpite — antes os dois sempre assumiam a branch padrão global, ignorando por completo esse campo da issue
- [x] Bug real de produção (visto numa issue de verdade): o executor externo escrevia `{message_file}`/`{config_dir}` DENTRO de `req.workdir` (o próprio repositório clonado). E pior, raiz do estrago maior: `sandboxed_shell` setava `HOME` igual a `ctx.workdir`, que em produção é um caminho RELATIVO (`workspace/task-x`) — um processo filho (visto com o opencode) que resolve `$HOME` contra o próprio cwd (já absoluto) duplicava o caminho (`workspace/task-x/workspace/task-x/...`), gravando cache/dados ali DENTRO do repositório. `git status` marcava tudo isso (arquivo de apoio, cache, um banco de ~40MB) como "mudança", bastando pra `verify()` aprovar a fase mesmo sem edição real — e tudo ia comitado pro PR, inclusive a chave da API em texto puro dentro do `opencode.jsonc`. Corrigido em duas camadas: `sandboxed_shell` (`oreia-core`) agora resolve `HOME`/`current_dir` pra absoluto sempre (conserta isso pra qualquer consumidor, inclusive o `builtin`), e o executor externo passou a escrever `{message_file}`/`{config_dir}` no diretório de temp do sistema, nunca no repositório, apagados ao fim da tentativa

## Pronto (v0.6)

- [x] [Benchmark e hardware recomendado](benchmark.md): orçamento de memória para rodar o orquestrador numa máquina pequena/sem GPU (ex. i5 + 8 GB) com o LLM numa máquina separada com GPU, e `scripts/benchmark.sh` para medir CPU/memória real por container (repouso e durante uma tarefa), cruzado com `data/escalation.jsonl`
- [x] Comparação real dos 4 executores externos (mesma tarefa, 5 issues separadas) em 2026-10-25: aider **destruiu o arquivo editado** (substituiu o conteúdo inteiro — descartado do projeto, não habilitado sem investigar a causa raiz) e goose, apesar de correto, foi ~3-5x mais lento — ambos comentados no Dockerfile. opencode e codex ficaram com builtin como os três usados neste projeto
- [x] [Containers por executor](executores.md#containers-por-executor): `container` em `[executors.<nome>]` faz esse executor rodar isolado, em imagem própria (`executors/opencode.Dockerfile`, `executors/codex.Dockerfile`, via `make executors-build`), lançado com `docker run --rm` pelo próprio `oreia-seca` (Docker-outside-of-Docker, `/var/run/docker.sock` montado só com `docker-compose.override.yml`, nunca no compose principal) — decoupla o Dockerfile do orquestrador das dependências de cada ferramenta (Node pro opencode/codex...). Opt-in explícito (sem `container`, continua rodando dentro do container do `oreia-seca` como antes); trade-off de segurança documentado em [Segurança](seguranca.md)
- [x] `/issue`, `/assign` e `/prs` [pelo Telegram](agentes.md#como-acionar) ganharam botões: sem argumentos, perguntam repositório/agente por botão e pedem o que não cabe num botão (título, descrição, branch base, número, texto de ajuste) por mensagem; `/prs` virou um cartão por PR com "✅ Merge"/"📝 Pedir ajuste" já apontando pro PR certo. O uso direto numa linha só continua igual (sem descrição — numa linha não dá pra separar título de descrição); o assistente do `/issue` sem argumentos pergunta título, descrição e branch base (`--base <branch>` no uso direto), já que é a descrição que o agente lê pra entender o que fazer, não o título, e nem toda issue vai pra `main`; o passo de branch lista as branches reais do repo como botão (via `GET /branches`), caindo pro texto livre se a chamada falhar
- [x] [px0](px0.md) ganhou **Status** (`/status`): fila, aprovações pendentes, trabalho ativo, papéis/modelos/agentes — reaproveitando a API REST que o `oreia-seca` já expunha (`/api/status`), atualizando ao vivo via `fetch()` em JS a cada 5s (sem recarregar a página, era `<meta refresh>` antes — piscava e reiniciava animações). Depois ganhou mais três painéis, sem acesso novo (reaproveita o `docker.sock`/mounts da Configuração): **atividade recente** (últimas tentativas de fase lidas de `data/escalation.jsonl` — executor, modelo, se passou, quanto tempo levou), um **gráfico de barras da duração** de cada tentativa (Chart.js, verde/vermelho), e **containers** (CPU/memória reais via `docker stats`). O doughnut de "modelos por estado" saiu — não agregava nada que a lista de modelos já não mostrasse
- [x] [px0](px0.md) ganhou **Configuração** (`/config`), opt-in: edita `oreia-seca.toml` (validado com `oreia-seca --check` via `docker exec` no container já rodando, antes de gravar) e `.env` (segredos mascarados na tela) pela web, reiniciando o(s) container(s) certo(s) depois de salvar. Exige `docker.sock` + os dois arquivos montados rw (`docker-compose.override.yml`) — escalada de confiança real (documentada em [Segurança](seguranca.md)): quem tiver acesso à tela passa a poder ler/trocar todos os segredos do projeto e reiniciar qualquer container. Validado em produção: toml inválido recusado sem tocar no arquivo real, toml válido grava e reinicia o oreia-seca, `.env` reinicia oreia-seca e o próprio px0 (confirmado via uptime do container) — inclusive o caso de reiniciar o container que está respondendo a própria requisição (resposta sai primeiro, restart agendado 1,5s depois)
- [x] Telegram registra os comandos reais no Telegram via `setMyCommands` ao iniciar — sem isso, o menu "/" mostrava o que tinha ficado registrado por qualquer outra aplicação que já usou o mesmo token do bot
- [x] [px0](px0.md) pra aprovação manual de PR: `localhost:7777` exige login (OAuth2 com a conta do Forgejo — login único, mesma sessão — ou usuário/senha manual, ambos filtrados por `PX0_ALLOWED_USERS`), lista os PRs abertos e clicar "Revisar" prepara e abre a sessão em `/review/` — sem terminal (`px0/launcher.py`, processo principal do container). O px0 em si não tem login nenhum, por isso não é mais exposto direto: só o launcher é publicado, e ele faz proxy reverso pro px0 (que escuta só em `127.0.0.1`, com `-base-path /review/`) exigindo sessão válida. px0 só fala HTTPS com o Gitea/Forgejo (sem opção de HTTP puro, confirmado testando) e a imagem do GHCR do guia oficial não existe, então: baixamos o binário real do GitHub (`px0/Dockerfile`), um sidecar `forgejo-tls` (Caddy, certificado próprio confiado pela imagem do px0) faz a ponte TLS, e o launcher reescreve o clone URL público do Forgejo (inacessível de dentro do container) com o token embutido. `scripts/bootstrap.sh` registra o app OAuth2 (`oreia-seca-px0`) automaticamente via API. Validado em produção ponta a ponta — inclusive o fluxo OAuth2 completo simulando um navegador real (sessão web do Forgejo → consentimento na primeira vez → código → token → sessão autenticada, sem digitar senha) e a allowlist recusando credencial válida fora da lista — falta só confirmar o clique de Approve/Merge na própria UI
- [x] `/api/check` + `crate::check` (`oreia-seca`): checagem automática do que mudou na configuração (modelo existe no servidor, token aponta pro usuário certo, binário/imagem do executor existe, Forgejo/Telegram respondem...) antes de salvar — usada pelo `/config` do px0, por `--check --live` na CLI e por `/check` no Telegram. Erro de rede nunca bloqueia (fica ⚠️); só config inválida, token rejeitado ou recurso ausente bloqueiam (❌). Veja [Checagem ao salvar](configuracao.md#checagem-ao-salvar)
- [x] **`oreia-seca setup --web`/`make setup-web` removido (2026-10-27).** Era um formulário HTML sem autenticação pensado só para `127.0.0.1`; o wizard de terminal e o arquivo de respostas (`make setup-answers`, pensado justamente para automação/agente de IA) já cobriam os mesmos casos de uso sem esse risco extra. `crates/oreia-seca/src/setup/web.rs` removido

## Pronto (v0.7)

- [x] [Servidor MCP](mcp.md): segunda via de trabalho, explícita e manual, pra um
  cliente MCP (Claude Code, Claude Desktop...) atuar como **P.O.** (`create_story`,
  `list_stories`, `get_story`, `get_pr`, `review_pr`) e **desenvolvedor sob demanda**
  (`checkout_issue`/`run_checks`/`submit_issue`/`release_issue` — assume uma issue
  livre ou que um agente não resolveu e codifica com o modelo do PRÓPRIO cliente; o
  OREIA cuida de git, checks e PR) e `dispatch_issue` (atalho pra via automática, com
  a opção de forçar um degrau da escalada). Nunca disputa a mesma issue com a via
  automática: posse persistente (`data/claims.json`, sobrevive a restart) e conta
  própria no Forgejo (`agent-mcp`, nunca um `[[agents]]`). `rmcp` 3.8.0 (SDK oficial),
  stdio por padrão e transporte HTTP streamable opcional (`[mcp].http`, Bearer
  `MCP_TOKEN` + allowlist de `Origin`). `/stats` passa a mostrar "resolvidas via MCP
  após falha", por agente original. Comentado/desligado por padrão — sem `[mcp]` no
  config, comportamento idêntico a antes desta feature existir

## Pronto (v0.8)

- [x] **TUI de `oreia-seca setup` (ratatui + crossterm), substituindo o antigo
  wizard de perguntas em sequência.** Sete áreas numa lista à esquerda e um
  painel à direita — **Status**, **px0**, **Forgejo**, **MCP**, **Telegram**,
  **LLM**, **Servidor** — liga/desliga e preenche cada uma sem precisar
  editar `.env`/`oreia-seca.toml` na mão:
  - **px0** não tem toggle de verdade (não existe `[px0]` no `.toml`, e o
    serviço já sobe sempre com `oreia-seca up`) — é um painel de acesso
    (`PX0_ALLOWED_USERS` + leitura das credenciais do bootstrap).
  - **Forgejo** (nova): url/url pública/repos/branch base/estilo de merge,
    criando o bloco `[forgejo]` se faltar.
  - **MCP**: liga/desliga `[mcp]` preservando comentários do resto do arquivo
    (`toml_edit`, em `setup/toml_edits.rs`), gera `MCP_TOKEN` sozinho ao ligar
    o transporte HTTP, e ganhou os campos avançados (`agent_user`, `workdir`,
    `max_review_rounds`, `bind`).
  - **LLM**: além de URL/modelos/chave, agora edita `context`/`temperature`
    de `[models.coder]`/`[models.chat]`.
  - **Servidor** (nova): host/porta da API, diretórios de dados/workspace,
    concorrência e timeout da fila — sempre com um valor efetivo (os
    defaults do `Config` se o bloco faltar no arquivo).
  - **Status** (nova): links de acesso (Forgejo/px0/API), estado dos
    containers e CPU/memória (via `docker compose ps`/`docker stats` no
    **host**, sem montar `docker.sock` em nada) e um resumo de
    `GET /api/status` quando o orquestrador responde — atualiza sozinha a
    cada ~5s enquanto a área estiver aberta.

  Campos numéricos (porta, concorrência, timeout, contexto, temperatura) são
  validados antes de salvar — um valor inválido mostra o erro na tela de
  resumo (`Ctrl+S`/`F2`) em vez de gravar algo quebrado. Só edita arquivos
  (sem chamar `/api/check` nem reiniciar containers) — mostra o que reiniciar
  manualmente no final. Os caminhos não interativos (`--answers`/flags)
  continuam existindo e ganharam `--px0-allowed`/`--mcp-enabled`/`--mcp-http`
  (os campos novos de Forgejo/Servidor/modelos são só da TUI por agora). Veja
  [Instalação](instalacao.md#tui-interativa)
- [x] **Bootstrap/up/down tentados de dentro da TUI, depois revertidos
  (2026-10-30 a 2026-10-31).** A área Status ganhou por um tempo as teclas
  `b` (ligar/desligar bootstrap ao salvar), `u` (`docker compose up`) e `d`
  (`docker compose down`), todas chamando `docker`/`docker compose` do
  próprio processo da TUI. Problema: quando `./oreia setup`/`make setup` cai
  para o container avulso do `oreia_seca_bin()` (sem `cargo` no host — o
  caso comum pra quem só vai usar o projeto), esse container não tem acesso
  ao Docker do host — as três ações falhavam, no começo com um erro cru de
  SO ("No such file or directory (os error 2)") sem explicação nenhuma. Uma
  tentativa intermediária só melhorou a mensagem (`status::docker_cli_error`
  detectando `ErrorKind::NotFound`); resolver de verdade exigiria
  Docker-outside-of-Docker completo (socket montado + plugin `compose` na
  imagem + traduzir os ~11 volumes relativos do `docker-compose.yml` pro
  caminho real do host via `OREIA_SECA_HOST_DIR`) — avaliado e descartado:
  risco real de quebrar a instalação nativa de quem já usa o projeto, pelo
  ganho de só não precisar digitar três comandos a mais. Decisão final:
  **`b`/`u`/`d` foram removidas** — a área Status voltou a ser só leitura
  (containers, CPU/memória, checklist, tecla `r` pra atualizar).
  Bootstrap/up/down são sempre `./oreia bootstrap`/`up`/`down` (ou `make`
  equivalente) no terminal, fora da TUI — único caminho confiável
  independente de ter `cargo` no host ou não. Documentado em
  [Instalação](instalacao.md#tui-interativa).
- [x] **Transporte HTTP do MCP não funcionava via `docker compose` (2026-10-31).**
  Três problemas empilhados, todos descobertos diagnosticando um "não estou
  conseguindo usar o MCP": (1) `docker-compose.yml` nunca publicava a porta
  3121 — corrigido com `${MCP_HTTP_PORT:-3121}:3121` no serviço `oreia-seca`;
  (2) o padrão de `[mcp].bind` (`127.0.0.1:3121`) é o loopback de DENTRO do
  container, nunca alcançável do host mesmo com a porta publicada — exige
  `bind = "0.0.0.0:3121"` nesse caso, documentado em
  [Servidor MCP](mcp.md#transporte-http-opcional); (3) o mais sério: `oreia-seca
  mcp` sempre terminava o processo inteiro (matando a tarefa HTTP junto) no
  instante em que o stdio batia EOF — o caso normal de um sidecar dedicado ao
  HTTP, sem nenhum cliente stdio do outro lado. Corrigido em `main.rs::run_mcp`:
  com `[mcp].http` ligado, o processo passa a esperar pelo handle do HTTP, não
  pelo stdio (que continua rodando em paralelo, pra quem de fato conectar por
  lá). Também adicionado `./oreia mcp-http`/`make mcp-http` — roda
  `oreia-seca mcp` em segundo plano dentro do container já ativo (`docker
  compose exec -d`); não é um serviço do compose, então não sobrevive a um
  restart do container, precisa rodar de novo depois de `./oreia up`.
  Verificado de ponta a ponta com um `initialize` real via `curl`.
- [x] **`/status` e `/config` saíram do px0 web** (`px0/launcher.py`), agora
  que a TUI cobre a mesma coisa — o px0 (`localhost:7777`) ficou só
  login + lista de PRs + proxy de revisão (`/review/`). Ganho de segurança
  real: o px0 não precisa mais de `docker.sock` nenhum (removido de
  `docker-compose.yml`/`docker-compose.override.example.yml`), tirando da
  tabela de riscos em [Segurança](seguranca.md) a linha de "acesso
  equivalente a root no host" que a tela de Configuração exigia. Veja
  [px0 → Status e configuração ficaram na TUI](px0.md#status-e-configuração-ficaram-na-tui-não-aqui)
- [x] **Instalação em um comando, assistente e painel de monitoramento (2026-11-12).**
  - **`./oreia install`** (e `oreia.cmd` no Windows, que só localiza o Git Bash):
    confere o Docker, constrói a imagem, abre o assistente, roda o bootstrap,
    sobe tudo, espera responder e mostra endereços e senha. `--answers
    ARQUIVO.toml` faz o mesmo sem perguntas. `./oreia` sem argumento instala na
    primeira vez e abre o painel nas seguintes. Novos: `doctor`, `saude`,
    `restart`, `update`, `reset [--all]`, `logs [serviço]`.
  - **Assistente de primeira instalação** (`setup/tui/wizard.rs`): provedor do
    LLM por lista (Ollama/LM Studio locais, outra máquina, OpenRouter, Gemini,
    OpenAI, outro, "depois"), modelo escolhido da lista do próprio servidor
    (com filtro), repositório + dono, Telegram opcional e revisão. Detecta
    portas ocupadas e desvia para a próxima livre; libera o dono no px0;
    desliga o canal do Telegram se não houver token.
  - **Painel**: áreas novas **Saúde** (Docker, orquestrador, Forgejo, Postgres,
    px0, LLM, Telegram, MCP — chamada real, latência, histórico, % no ar, e o
    que fazer) e **Uso** (tokens, custo, taxa de sucesso e tempo por modelo e
    por papel, série diária, últimas execuções — de `escalation.jsonl`);
    **Status** ganhou fila, mini-gráficos de CPU/memória e saúde dos containers.
    Coleta num laço em segundo plano (`setup/tui/monitor.rs`) — a tela não
    trava mais ~2s a cada `docker stats`.
  - **`u`/`d`/`b` voltaram, do jeito certo.** O problema de 2026-10-31 (TUI num
    container sem acesso ao Docker do host) foi resolvido em duas partes: para
    LER o estado, o container avulso recebe o `docker.sock` e o painel usa só
    `docker ps`/`docker stats` filtrando pela label do projeto — sem o plugin
    `compose` e sem traduzir caminhos de volume; para AGIR, a TUI grava a ação
    em `.oreia-action` e sai, e o launcher executa no host e reabre o painel.
    Nenhum `docker compose` roda de dentro da TUI.
  - **Portabilidade**: `scripts/bootstrap.sh` sem `mapfile` nem expansão de
    array vazio (roda no bash 3.2 do Mac); cliente docker da imagem por
    arquitetura (`uname -m`, serve em ARM); `.dockerignore` deixa `volumes/` de
    fora do contexto de build; no Linux o binário devolve a posse do `.env`
    gravado pelo container (`OREIA_CHOWN`); `docker-compose.yml` e o launcher
    compartilham a imagem `oreia-seca:local`.
  - `oreia-seca saude` (texto, código de saída 1 com serviço fora do ar) para
    script/cron. Respostas novas no modo não interativo: `[forgejo].owner_user`,
    `[forgejo].repos`, `[ports]` e as flags `--forgejo-owner`/`--forgejo-repos`.
  - Verificado com uma instalação completa de verdade (`./oreia install
    --answers`, projeto Compose e portas separados) e 281 testes, incluindo a
    renderização de cada tela num terminal simulado. Veja
    [Instalação](instalacao.md), [Painel](painel.md) e
    [Solução de problemas](problemas.md).

## Próximos passos

1. **Validar ponta a ponta com modelo real.** Rodar `./oreia install` contra um servidor de verdade, criar uma issue pequena e ajustar prompts, `max_steps` e a escalada com base no que os modelos acertarem e errarem.
2. **Planner que cria issues.** `/feature` com Forgejo configurado → o planner quebra em issues com as labels certas (back/front), contrato primeiro.
3. **Isolamento por execução do `builtin`.** Executores externos com `container` configurado já rodam isolados (veja v0.6 acima); falta o mesmo para o executor `builtin` (hoje ainda dentro do container do `oreia-seca`, por diretório de tarefa) — rodar cada tentativa num container efêmero (ou LXC no Proxmox).
4. **Previews por PR.** Validar o template `templates/app-repo` com a stack do seu sistema e devolver o link do preview no Telegram (`/preview <n>`).
5. **Limites de iteração entre agentes.** Contar quantas vezes um PR voltou para ajustes e pedir sua intervenção a partir de N.
6. **Portar o que falta do OREIA-SECA** (veja [vindo-do-oreia-seca.md](vindo-do-oreia-seca.md)).
7. **MCP com `oreia-seca` remoto.** Hoje `checkout_issue` exige que o cliente MCP enxergue `[mcp].workdir` (mesma máquina, ou volume montado); uma fase 2 cobriria clonar localmente e publicar só o branch pronto via `submit_issue`.
