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

## Próximos passos

1. **Validar ponta a ponta com modelo real.** Rodar `make setup` contra um servidor de verdade, criar uma issue pequena e ajustar prompts, `max_steps` e a escalada com base no que os modelos acertarem e errarem.
2. **Planner que cria issues.** `/feature` com Forgejo configurado → o planner quebra em issues com as labels certas (back/front), contrato primeiro.
3. **Isolamento por execução.** Rodar cada tarefa num container efêmero (ou LXC no Proxmox) em vez de diretórios no mesmo container.
4. **Previews por PR.** Validar o template `templates/app-repo` com a stack do seu sistema e devolver o link do preview no Telegram (`/preview <n>`).
5. **Limites de iteração entre agentes.** Contar quantas vezes um PR voltou para ajustes e pedir sua intervenção a partir de N.
6. **Portar o que falta do OREIA-SECA** (veja [vindo-do-oreia-seca.md](vindo-do-oreia-seca.md)).
