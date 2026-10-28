# Instalação

Depois de `make bootstrap` (Forgejo) a única configuração que falta é
Telegram + LLM + Forgejo no `.env`/`oreia-seca.toml`. `oreia-seca setup`
faz isso de duas formas — escolha a que preferir, as duas chegam no mesmo lugar:

| Forma | Comando | Quando usar |
|---|---|---|
| Wizard de terminal | `make setup` | Padrão: pergunta tudo, uma coisa por vez |
| Arquivo de respostas | `make setup-answers` | Automatizar, repetir em várias máquinas, ou deixar um agente de IA preencher |

As duas só **coletam** as respostas de um jeito diferente (terminal ou um
arquivo TOML). Quem decide o que fazer com elas é sempre a mesma função
(`apply_answers`, em `crates/oreia-seca/src/setup/mod.rs`): grava o `.env` e,
se você pedir, roda `scripts/bootstrap.sh`. Isso quer dizer que o que está
documentado aqui vale para as duas igualmente.

## Os dois arquivos: `.env` e `oreia-seca.toml`

`oreia-seca.toml` define **o quê** (modelos, papéis, agentes, repositórios do
Forgejo) — nunca guarda segredo, só o *nome* de uma variável (`base_url_env`,
`api_key_env`, `owner_token_env`...). `.env` guarda **os valores** de verdade
dessas variáveis. Por isso o `.toml` poderia até ir para o git (não vai, mas
poderia): sozinho, ele não vaza nada. Veja [Configuração](configuracao.md)
para a referência campo a campo.

Nenhum dos dois é versionado — os templates que ficam no git são
`.env.example` e `oreia-seca.example.toml`; os arquivos de verdade são cópias
deles, preenchidas pelo `setup` (ou pelo `bootstrap`, se o `.env` ainda não
existir).

## Começar do zero (reinstalação limpa)

Três coisas guardam estado de uma instalação anterior, além dos dois arquivos
acima:

| O quê | Por quê existe |
|---|---|
| `volumes/` | dados reais do Postgres, do Forgejo (contas, repos, issues) e do próprio `oreia-seca` (tarefas, sessões) — bind mount, não volume nomeado do Docker |
| `tls/` | certificados internos do sidecar `forgejo-tls` (usado pelo px0) — o bootstrap gera de novo sozinho se faltar |
| `docker-compose.override.yml` | **opt-in**, só existe se você configurou containers por executor ou a edição de configuração do px0 (veja [Executores](executores.md) e [px0](px0.md)) |

Para resetar de verdade:

```bash
docker compose down
rm -f .env oreia-seca.toml docker-compose.override.yml
rm -rf volumes tls
```

Depois, qualquer uma das duas formas de `setup` (abaixo) recria `.env` e
`oreia-seca.toml` a partir dos exemplos. Se for só o Forgejo que você quer
resetar (Telegram/LLM já configurados e você não quer tocar neles), rode
`make bootstrap`/`./oreia bootstrap` direto — ele mesmo cria `.env`/`.toml`
que estiverem faltando antes de seguir.

**Duas pegadinhas reais, de quem já passou por isso:**

- Se você tinha `docker-compose.override.yml` (containers por executor ou
  `/config` do px0) e só apagou `.env`, o `docker compose up` quebra com
  `required variable DOCKER_GID is missing a value` — esse arquivo é
  carregado automaticamente junto do `docker-compose.yml`, mesmo sem pedir.
  Ou apaga os dois junto, ou define `DOCKER_GID`/`OREIA_SECA_HOST_DIR` de
  novo no `.env` novo.
- Depois do bootstrap, `PX0_ALLOWED_USERS` continua vazio (o padrão) —
  **vazio bloqueia todo mundo**, mesmo com o login OAuth2 funcionando
  perfeitamente ("Client ID not registered"/"conta não autorizada" na
  tela). Preencha com o seu usuário do Forgejo (o de `FORGEJO_OWNER_USER`)
  antes de tentar entrar em `localhost:7777`. Veja
  [px0 → Allowlist](px0.md#autenticação-e-por-que-ela-também-protege-o-px0).

## Sem `make`

`make` não vem instalado em todo sistema (não vem, por exemplo, no Git Bash
puro do Windows). Todo comando `make <x>` deste documento tem o mesmo efeito
rodando `./oreia <x>` — um script bash na raiz do projeto, sem outra
dependência nova. `./oreia help` lista tudo. As duas formas convivem: use a
que preferir, ou as duas, nunca fazem coisas diferentes.

Preferir chamar o binário Rust direto, sem bash nem make? `status`, `up` e
`down` também existem como subcomandos do próprio `oreia-seca`
(`cargo run -p oreia-seca -- status`, por exemplo) — funciona em qualquer SO
onde `cargo` e Docker já estejam instalados, nem precisa do script.

## Está rodando? Portas ocupadas?

```bash
make status          # ou: ./oreia status / cargo run -p oreia-seca -- status
```

Mostra os containers (`docker compose ps`) e confere se o orquestrador
(`/health`) e o Forgejo (`/api/healthz`) estão respondendo. `make down` /
`./oreia down` para tudo quando não estiver usando.

Se `make bootstrap`/`./oreia bootstrap` ou `make up`/`./oreia up` acusarem
porta ocupada (3000, 2222 ou 3120 já usadas por outro container ou processo
nessa máquina), mude `FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT` ou
`OREIA_SECA_PORT` no `.env` para portas livres e rode de novo — o bootstrap
confere antes de tentar subir, em vez de deixar o Docker falhar com um erro
cru. A comunicação entre os containers (webhook do Forgejo, `[forgejo].url`
em `oreia-seca.toml`, etc.) continua funcionando sem nenhuma mudança: essas
variáveis só afetam a porta que você acessa **de fora**, nunca a rede
interna do Docker.

> 💡 **Certificados internos do px0 (`tls/`):** O `make bootstrap` / `./oreia bootstrap` confere e gera automaticamente os certificados internos em `tls/` necessários para o build da interface web de PRs (`px0`). Se precisar gerá-los ou regenerá-los manualmente em outra máquina, veja [docs/px0.md](px0.md#certificados-tls-internos-tls-e-regeneração).

## Wizard de terminal

```bash
make setup
```

Pergunta, em ordem: token do Telegram (Enter pula — desativa o canal), seu ID
numérico, a URL do LLM, a chave da API (só se a URL não parecer de rede
local), o modelo para código e para conversa direta (lista os modelos do
servidor quando consegue), e por fim se quer rodar o bootstrap do Forgejo
agora. No final mostra um resumo do que foi gravado.

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

Sem nenhum desses argumentos, `oreia-seca setup` cai no wizard de terminal
interativo.

## Só o LLM

Se Telegram e Forgejo já estão configurados e só o LLM mudou (trocou de
servidor, por exemplo), `oreia-seca setup llm` (ou `make llm-setup`) continua
existindo — é o que o `setup` unificado chama por baixo para essa parte. Veja
[Modelos locais](modelos-locais.md).

## E o Forgejo?

`[forgejo].bootstrap = true` (ou escolher "sim" no wizard) roda exatamente
`scripts/bootstrap.sh`: sobe Postgres e
Forgejo, cria o seu usuário e os dos agentes, gera os tokens, cria o
repositório, as labels, o webhook e protege o `main`. Veja
[Agentes](agentes.md) e o próprio script para o que ele espera já estar no
`oreia-seca.toml` (`[forgejo].repos`, principalmente).

Se preferir não rodar automaticamente, deixe `bootstrap = false` (o padrão) e
rode `make bootstrap` manualmente quando quiser — nada no `setup` obriga a
fazer os dois juntos.
