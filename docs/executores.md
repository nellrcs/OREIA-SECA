# Executores e escalada

Por padrão, cada fase roda no executor **builtin**: o loop agêntico original
(modelo → ações `<action>` → resultado), com o modelo do papel que a tarefa
usa. Isso é o suficiente para a maioria dos casos e é exatamente o que
acontece quando você não configura nada a mais neste documento.

Para papéis onde vale a pena tentar mais de uma vez — um modelo mais barato
primeiro, uma ferramenta especializada depois, um modelo mais caro só se
precisar — dá para configurar uma **escada de escalada**: uma lista de
degraus, cada um com seu executor e modelo, tentados em ordem até um passar
na verificação.

## Executor builtin vs. externo

| | `builtin` | externo (`aider`, `opencode`, `goose`, `codex`, ou outro) |
|---|---|---|
| O que é | O loop de ações `<action>` deste projeto | Uma CLI de terceiros, chamada por um template de comando |
| Como edita código | Via ações `file_write`/`file_read`/`shell` | Do jeito da própria ferramenta |
| Commit | Nunca (o orquestrador commita) | Nunca — os presets desligam o auto-commit da ferramenta (ex.: `--no-auto-commits` no Aider); o orquestrador commita |
| Configuração | Nenhuma | `[executors.<nome>]` (opcional: os quatro de cima já têm preset) |

Um `[executors.<nome>]` só precisa do `command`, um template com estes
placeholders — **mas atenção**: nem toda ferramenta aceita `{base_url}`/
`{api_key}` como flag (veja a tabela de presets abaixo); o placeholder existe
para você usar na posição que a SUA ferramenta espera, não é garantia de que
toda CLI tenha uma flag `--api-key`.

| Placeholder | Vira |
|---|---|
| `{model}` | o nome do modelo no provedor (`model` em `[models.*]`) |
| `{base_url}` | o `base_url` do modelo |
| `{api_key}` | a chave do modelo (`api_key_env`), vazia se não houver |
| `{message_file}` | caminho **absoluto** de um arquivo com a instrução da fase (e o motivo da falha anterior, se houver) |
| `{test_cmd}` | o `test_cmd` da escalada, para a ferramenta rodar a própria verificação, se souber |

```toml
[executors.aider]
command = "aider --model openai/{model} --openai-api-base {base_url} --openai-api-key {api_key} --no-auto-commits --yes-always --message-file {message_file}"
```

A chave do modelo nunca aparece em mensagens nem no `data/escalation.jsonl`:
o comando de fato executado leva a chave, mas o texto registrado/mostrado
troca `{api_key}` por `***`, onde quer que `{api_key}` apareça no template.

### Os quatro presets embutidos — testados de verdade, não só pelo `--help`

As flags mudam entre versões, e a primeira versão destes presets (baseada só
na documentação de cada ferramenta) estava errada pra três das quatro: só o
**aider** de fato tem `--openai-api-base`/`--openai-api-key`. Os outros três
não têm essas flags — cada um resolve autenticação/endpoint do próprio jeito.
Isso foi verificado com três testes que rodam de verdade (não só leem
`--help`), todos contra a API real da OpenRouter com
`deepseek/deepseek-v4.1-flash-1017` **e** `google/gemini-2.5-flash-lite`, em
2026-10-25:
- `live_presets_actually_work_against_openrouter` (`executor/external.rs`) —
  os 4 presets externos, via `ExternalExecutor::run`.
- `live_openai_compat_works_with_deepseek_and_gemini` (`oreia-llm/src/openai.rs`)
  — só o provedor HTTP que o `builtin` usa por baixo.
- `live_builtin_executor_runs_the_agentic_loop_with_deepseek_and_gemini`
  (`executor/builtin.rs`) — o `builtin` de ponta a ponta: o loop agêntico
  inteiro (modelo → parse de `<action>` → execução via `ActionRegistry`), não
  só a chamada HTTP.

Rode `OREIA_SECA_LIVE_EXTERNAL_TESTS=1 OPENROUTER_KEY=<sua chave>` + um desses
nomes de teste depois de uma atualização das ferramentas; os dois últimos não
precisam de CLI nenhuma instalada, só o primeiro precisa — nesse caso,
`scripts/live_test_setup.sh` instala as 4 num container descartável. Texto
exato dos presets: `BUILTIN_PRESETS` em `crates/oreia-seca/src/executor/external.rs`.

| Ferramenta | Como autentica / aponta pro `base_url` | Pegadinha |
|---|---|---|
| **aider** | `--openai-api-base {base_url} --openai-api-key {api_key}` — as únicas das quatro que existem de fato | Nenhuma encontrada. Funciona com os dois modelos testados |
| **opencode** | `XDG_CONFIG_HOME={config_dir} opencode run -m oreiaseca/{model}` — não tem `--api-base`/`--api-key`, e os provedores que o opencode já conhece (ex. `openrouter`) validam o modelo contra o catálogo *próprio* dele (cacheado de models.dev), não contra a API de verdade | Tentar usar o provedor `openrouter` já conhecido do opencode falhava com `google/gemini-2.5-flash-lite` ("Model not found", escondido atrás de um erro genérico sem `--log-level debug") porque esse catálogo não lista esse modelo, mesmo a OpenRouter servindo ele de verdade. **Corrigido**: `run()` agora escreve um `opencode.jsonc` (em `{config_dir}`, no diretório de temp do sistema — nunca dentro do repositório clonado, e apagado ao fim da fase) definindo um provedor `oreiaseca` ad-hoc via o pacote `@ai-sdk/openai-compatible`, apontando pro `{base_url}`/`{model}` exatos. Isso não passa pelo catálogo, então funciona com qualquer modelo que o endpoint sirva de verdade — e de brinde deixou de depender especificamente da OpenRouter (igual aider/goose/codex agora). **Bug relacionado corrigido no mesmo commit**: `{message_file}` e esse `opencode.jsonc` viviam antes dentro de `req.workdir` (o próprio repositório) — `git status` via esses arquivos de apoio como "mudança" e isso bastava pra `verify()` aprovar a fase mesmo sem o modelo ter editado nada de real, além de arriscar comitar a chave da API (em texto puro, dentro do `opencode.jsonc`) pro PR. Agora ficam fora do repositório, no temp do sistema, e são removidos ao fim da tentativa |
| **goose** | `OPENAI_HOST="${BASE_URL%/v1}"` (sem o `/v1` final — a CLI completa o resto sozinha) + `GOOSE_PROVIDER=openai GOOSE_MODEL={model}` | Se você passar `{base_url}` direto (com `/v1`), a CLI monta `.../v1/v1/chat/completions` e recebe 404. Funciona com os dois modelos testados |
| **codex** | `OPENAI_BASE_URL` é **ignorado** (a CLI sempre bate em `api.openai.com` por padrão) — o jeito que funciona é definir um provedor ad-hoc via `-c model_providers.custom.base_url={base_url} -c model_providers.custom.env_key=<VAR> -c model_provider=custom`, com `<VAR>={api_key}` | Fácil assumir que `OPENAI_BASE_URL`/`OPENAI_API_KEY` bastam (é o padrão de várias outras ferramentas) — aqui não funciona, falha silenciosamente com 401 contra o endpoint errado. Funciona com os dois modelos testados |

Instalação de cada um (Dockerfile, bloco "Executores externos") e permissões
no container: veja os comentários lá — resumo rápido:
- **opencode** e **goose** instalam via `curl | bash` em `$HOME` (de `root`,
  durante o build) — `/root` é `700`, então o binário fica inacessível pro
  usuário `oreia-seca` que roda o processo final, a não ser que seja copiado
  pra `/usr/local/bin` depois (o Dockerfile já faz isso pro opencode).
- **goose** especificamente: o instalador oficial também tenta um passo de
  configuração interativa que precisa de um terminal (`/dev/tty`) — quebra
  num build normal. O bloco do Dockerfile baixa e extrai o binário direto,
  pulando esse passo.
- **aider** via `pip`: Debian Bookworm recusa instalar fora de venv sem
  `--break-system-packages` (PEP 668).
- **codex** via `npm`: exige Node/npm, que não vêm na imagem base.

Por padrão, só o **opencode** vem habilitado no Dockerfile (é o único usado
neste projeto até agora); os outros três estão documentados e prontos — é só
descomentar o bloco.

## Escalada por papel

```toml
[escalation.dev-back]
steps = [
  { executor = "builtin", model = "qwen-coder-7b" },
  { executor = "builtin", model = "qwen-coder-32b" },
  { executor = "opencode", model = "planner-remoto", cost_per_mtok_in = 0.5, cost_per_mtok_out = 1.5 },
]
ask_before_step = 3
max_cost_usd = 2.0
test_cmd = "cargo test"
```

- **Papel sem entrada em `[escalation]`** usa um degrau só — o modelo do
  próprio papel, sem `test_cmd`. É o comportamento de antes da escalada
  existir.
- **Verificação.** Depois de cada tentativa: se o executor falhou, se nada
  mudou (quando a instrução pedia mudança de código) ou se `test_cmd` saiu
  com erro, a fase passa para o degrau seguinte. Sem `test_cmd`, só as duas
  primeiras checagens valem — um modo mais fraco, vale configurar um comando
  de teste sempre que possível.
- **Isolamento entre tentativas.** Uma tentativa que falha é desfeita antes
  do degrau seguinte (`git checkout -- . && git clean -fd`); o que os degraus
  anteriores *aprovados* já fizeram continua, porque fica commitado
  localmente a cada degrau que passa. No final, esses commits de checkpoint
  são achatados num só, então o PR sai com um commit por fase de trabalho.
- **Contexto de falha.** O degrau seguinte recebe o que deu errado: a saída
  de teste (últimas linhas), o diff tentado (truncado) e o erro.
- **`ask_before_step`** (1-based): a partir deste degrau, o Telegram pergunta
  antes de tentar, com o custo estimado. Sem resposta ou rejeitado, a fase
  falha.
- **`max_cost_usd`**: soma o custo estimado (tokens × `cost_per_mtok_*` de
  cada degrau) por tarefa. Ao passar do teto, a escalada para e a fase falha
  se nenhum degrau anterior tiver passado.
- Isso tudo é **por fase**: fases já aprovadas não são refeitas, e cada
  tarefa nova recomeça do degrau 1.
- O PR final tem uma seção **"Escalada"** listando, por fase, qual degrau
  resolveu.

## `/stats [papel]`

Agrega `data/escalation.jsonl` (uma linha por tentativa): por papel, quanto
cada degrau resolveu, custo total e médio por tarefa.
