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
placeholders:

| Placeholder | Vira |
|---|---|
| `{model}` | o nome do modelo no provedor (`model` em `[models.*]`) |
| `{base_url}` | o `base_url` do modelo |
| `{api_key}` | a chave do modelo (`api_key_env`), vazia se não houver |
| `{message_file}` | caminho de um arquivo com a instrução da fase (e o motivo da falha anterior, se houver) |
| `{test_cmd}` | o `test_cmd` da escalada, para a ferramenta rodar a própria verificação, se souber |

```toml
[executors.aider]
command = "aider --model openai/{model} --openai-api-base {base_url} --openai-api-key {api_key} --no-auto-commits --yes-always --message-file {message_file}"
```

As flags mudam entre versões — confira a documentação atual da ferramenta
antes de depender disso em produção. Os presets embutidos (sem precisar
declarar `[executors.*]`) cobrem `aider`, `opencode`, `goose` e `codex` com um
comando razoável; veja `oreia-seca.example.toml` para o texto exato.

A chave do modelo nunca aparece em mensagens nem no `data/escalation.jsonl`:
o comando de fato executado leva a chave, mas o texto registrado/mostrado
troca `{api_key}` por `***`.

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
