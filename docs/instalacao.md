# Instalação

Depois de `make bootstrap` (Forgejo) a única configuração que falta é
Telegram + LLM + Forgejo no `.env`/`oreia-seca.toml`. `oreia-seca setup`
faz isso de três formas — escolha a que preferir, todas chegam no mesmo lugar:

| Forma | Comando | Quando usar |
|---|---|---|
| Wizard de terminal | `make setup` | Padrão: pergunta tudo, uma coisa por vez |
| Página web local | `make setup-web` | Prefere um formulário a responder no terminal |
| Arquivo de respostas | `make setup-answers` | Automatizar, repetir em várias máquinas, ou deixar um agente de IA preencher |

As três só **coletam** as respostas de um jeito diferente (terminal, HTML, ou
um arquivo TOML). Quem decide o que fazer com elas é sempre a mesma função
(`apply_answers`, em `crates/oreia-seca/src/setup/mod.rs`): grava o `.env` e,
se você pedir, roda `scripts/bootstrap.sh`. Isso quer dizer que o que está
documentado aqui vale para as três igualmente.

## Wizard de terminal

```bash
make setup
```

Pergunta, em ordem: token do Telegram (Enter pula — desativa o canal), seu ID
numérico, a URL do LLM, a chave da API (só se a URL não parecer de rede
local), o modelo para código e para conversa direta (lista os modelos do
servidor quando consegue), e por fim se quer rodar o bootstrap do Forgejo
agora. No final mostra um resumo do que foi gravado.

## Página web local

```bash
make setup-web
```

Sobe um formulário em `http://127.0.0.1:3121/` com os mesmos campos do wizard
de terminal. Ao enviar, grava o `.env`, roda o bootstrap se você marcou a
caixa, mostra o resultado na própria página e **desliga o servidor** — não
fica rodando depois.

> ⚠️ **Sem autenticação.** Pensada para `127.0.0.1` (o padrão): só quem está
> na própria máquina acessa. Se precisar ligar em outro host
> (`oreia-seca setup --web --host 0.0.0.0`, por exemplo configurando um
> servidor remoto), qualquer um que alcançar a porta 3121 consegue gravar o
> `.env` dessa máquina enquanto o formulário estiver no ar — use só numa rede
> que você confia, nunca exposto na internet.

## Arquivo de respostas (ou flags) — para scripts e agentes de IA

Copie [`setup.answers.example.toml`](../setup.answers.example.toml) para
`setup.answers.toml`, preencha e rode:

```bash
make setup-answers                       # usa setup.answers.toml
make setup-answers ANSWERS=outro.toml    # ou outro caminho
```

Sem terminal interativo nem navegador — é o caminho para automatizar ou para
dar a um agente de IA (um que já tenha acesso ao shell deste projeto, como o
Claude Code). Peça para ele preencher o arquivo com o que você quer e rodar o
comando acima; ele não precisa adivinhar nada, o arquivo abaixo documenta
cada campo:

```toml
[telegram]
token = ""              # crie com o @BotFather; em branco desativa o Telegram
allowed = ""             # seu ID numérico (ou vários, separados por vírgula)

[llm]
url = ""                 # ex.: "http://<ip>:11434/v1" (Ollama), "https://openrouter.ai/api/v1"
model = ""               # modelo para planner/dev-*/executor
chat_model = ""          # modelo para a conversa direta; em branco usa o mesmo de "model"
api_key = ""             # só se o servidor exigir; em branco = sem autenticação

[forgejo]
bootstrap = false        # true roda scripts/bootstrap.sh (precisa de Docker, demora um pouco)
```

Regras:
- Uma seção inteira em branco (ou ausente) não muda nada do que já está
  configurado para ela — `oreia-seca setup --answers` só sobrescreve o que
  você de fato preencheu.
- `llm.url` e `llm.model` são um par: dar só um dos dois é erro (`validate`
  recusa antes de gravar qualquer coisa).
- Campo que não existe no esquema (erro de digitação) é rejeitado na hora de
  ler o arquivo, não ignorado silenciosamente.

### Só com flags, sem arquivo

Para os casos mais comuns, as mesmas respostas também dão para passar direto
na linha de comando — útil num script de uma linha só:

```bash
cargo run -p oreia-seca -- setup \
  --llm-url http://192.168.1.50:11434/v1 --llm-model qwen2.5-coder:14b \
  --telegram-token "123:ABC..." --telegram-allowed 987654321 \
  --forgejo-bootstrap
```

| Flag | Equivale a |
|---|---|
| `--answers ARQUIVO.toml` | ler esse arquivo primeiro (flags abaixo sobrescrevem o que ele trouxer) |
| `--telegram-token`, `--telegram-allowed` | `[telegram]` |
| `--llm-url`, `--llm-model`, `--llm-chat-model`, `--llm-api-key` | `[llm]` |
| `--forgejo-bootstrap` | `[forgejo].bootstrap = true` |

Com `--answers` e flags juntos, as flags ganham — dá para ter um arquivo-base
e sobrescrever um campo pontual sem editá-lo.

Sem nenhum desses argumentos (e sem `--web`), `oreia-seca setup` cai no
wizard de terminal interativo.

## Só o LLM

Se Telegram e Forgejo já estão configurados e só o LLM mudou (trocou de
servidor, por exemplo), `oreia-seca setup llm` (ou `make llm-setup`) continua
existindo — é o que o `setup` unificado chama por baixo para essa parte. Veja
[Modelos locais](modelos-locais.md).

## E o Forgejo?

`[forgejo].bootstrap = true` (ou marcar a caixa na página web, ou escolher
"sim" no wizard) roda exatamente `scripts/bootstrap.sh`: sobe Postgres e
Forgejo, cria o seu usuário e os dos agentes, gera os tokens, cria o
repositório, as labels, o webhook e protege o `main`. Veja
[Agentes](agentes.md) e o próprio script para o que ele espera já estar no
`oreia-seca.toml` (`[forgejo].repos`, principalmente).

Se preferir não rodar automaticamente, deixe `bootstrap = false` (o padrão) e
rode `make bootstrap` manualmente quando quiser — nada no `setup` obriga a
fazer os dois juntos.
