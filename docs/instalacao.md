# Instalação

Um comando instala tudo. Ele faz algumas perguntas, sobe o Forgejo, cria as
contas e o repositório, e deixa os agentes prontos para trabalhar.

```bash
./oreia install        # Linux, Mac, WSL, Git Bash
oreia.cmd install      # Windows (cmd ou PowerShell)
```

Você não precisa editar `.env` nem `oreia-seca.toml` na mão — nem na
instalação, nem depois.

- [Requisitos](#requisitos)
- [Passo a passo](#passo-a-passo)
- [O que o assistente pergunta](#o-que-o-assistente-pergunta)
- [Depois de instalar](#depois-de-instalar)
- [Instalação automatizada](#instalação-automatizada) (script, CI, agente de IA)
- [Comandos do dia a dia](#comandos-do-dia-a-dia)
- [Atualizar](#atualizar)
- [Recomeçar do zero](#recomeçar-do-zero)
- [Como funciona por baixo](#como-funciona-por-baixo)

Algo não subiu? Rode `./oreia doctor` e veja [Solução de problemas](problemas.md).

## Requisitos

Só o **Docker**. O programa é compilado e roda dentro dele — não precisa
instalar Rust, Python, Node, `make` nem nada mais.

| Sistema | O que instalar |
|---|---|
| **Windows 10/11** | [Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/) e [Git for Windows](https://git-scm.com/download/win) (você já tem, se clonou o projeto com `git` — é ele que traz o bash que o instalador usa) |
| **Mac** (Intel ou Apple Silicon) | [Docker Desktop](https://docs.docker.com/desktop/setup/install/mac-install/) |
| **Linux** | Docker Engine com o plugin Compose: `curl -fsSL https://get.docker.com \| sh`, depois `sudo usermod -aG docker $USER` e entre de novo na sessão |

Além disso:

- Um **modelo de IA** acessível — ele nunca roda junto com o OREIA-SECA. Pode
  ser o Ollama ou o LM Studio no seu próprio computador, noutra máquina da
  rede, ou um serviço como OpenRouter, Gemini ou OpenAI. Dá para instalar sem
  e escolher depois. Veja [Modelos locais](modelos-locais.md).
- Alguns GB de disco para as imagens do Docker e os dados. Para medir o
  consumo real de CPU e memória na sua máquina, veja [Benchmark](benchmark.md).
- Opcional: um bot do **Telegram**, para acompanhar e aprovar pelo celular.

## Passo a passo

1. **Baixe o projeto** e entre na pasta:

   ```bash
   git clone <url-do-repositório> oreia-seca
   cd oreia-seca
   ```

2. **Abra o Docker** (Docker Desktop no Windows/Mac; no Linux o serviço já
   fica rodando).

3. **Rode o instalador:**

   ```bash
   ./oreia install        # no Windows, fora do Git Bash: oreia.cmd install
   ```

O instalador passa por quatro etapas e mostra cada uma na tela:

| Etapa | O que acontece | Quanto demora |
|---|---|---|
| 1. Docker e programa | confere o Docker e constrói a imagem do OREIA-SECA | 5–15 min **só na primeira vez** (compila o programa); depois, segundos |
| 2. Configuração | abre o [assistente](#o-que-o-assistente-pergunta) | o tempo de responder 4 perguntas |
| 3. Forgejo | sobe o Postgres e o Forgejo, cria sua conta, as dos agentes, os tokens, o repositório, as labels, o webhook e protege o branch `main` | 1–3 min |
| 4. Serviços | sobe o orquestrador e o px0 e espera responderem | 1–2 min |

No final aparece o resumo — guarde a senha (ela também fica no `.env`):

```
✅ OREIA-SECA instalado e rodando.

  Forgejo (seu git)       http://localhost:3000
                          usuário: dono   senha: 4f9c…
  px0 (revisão de PRs)    http://localhost:7777   (entra com a mesma conta do Forgejo)
  Painel                  ./oreia
```

Se algo falhar no meio (internet caiu, porta ocupada), **é seguro rodar
`./oreia install` de novo**: ele continua de onde parou, aproveitando o que
já foi respondido e criado.

## O que o assistente pergunta

Uma tela por pergunta. `Enter` avança, `Esc` volta, `Ctrl+C` sai sem instalar
nada.

**1. De onde vem o modelo de IA.** Uma lista para escolher com as setas:

| Opção | Endereço que ele preenche |
|---|---|
| Ollama neste computador | `http://host.docker.internal:11434/v1` |
| LM Studio neste computador | `http://host.docker.internal:1234/v1` |
| Ollama ou LM Studio em outra máquina da rede | `http://<ip>:11434/v1` (você troca o `<ip>`) |
| OpenRouter · Google Gemini · OpenAI | o endereço do serviço; pede a chave da API |
| Outro servidor compatível com OpenAI | você digita |
| Configurar depois | pula; instala tudo e você escolhe o modelo mais tarde no painel |

> `host.docker.internal` quer dizer "este computador, visto de dentro do
> Docker". Não troque por `localhost` — dentro de um container, `localhost` é
> o próprio container.

**2. Qual modelo.** O assistente consulta o servidor e mostra os modelos que
ele tem; digite para filtrar (o OpenRouter lista centenas). Se o servidor não
responder, dá para digitar o nome do modelo e seguir assim mesmo — a área
**Saúde** do painel mostra depois se conectou.

**3. Seu projeto.** O nome do repositório, no formato `dono/repo` (ex.:
`minha-org/meu-sistema`), e o seu nome de usuário no Forgejo (padrão: `dono`).
O instalador cria os dois. A senha é gerada sozinha.

**4. Telegram (opcional).** O token do bot (crie com o
[@BotFather](https://t.me/BotFather): `/newbot`) e o seu ID numérico. Deixe
tudo em branco para pular. Não sabe o ID? Deixe em branco: depois de
instalar, mande "oi" ao bot e ele responde com o número.

**5. Revisão.** Mostra o que vai ser gravado e os endereços finais. Se
alguma porta padrão (3000, 2222, 3120, 7777) já estiver em uso nesta máquina,
o assistente **escolhe sozinho a próxima livre** e avisa aqui.

O que o assistente decide por você (e dá para mudar depois, no painel):

- o dono já entra liberado no px0 (`PX0_ALLOWED_USERS`);
- sem token, o canal do Telegram fica desligado em vez de avisar a cada boot;
- o mesmo modelo serve para planejar, programar e conversar;
- três agentes: `dev-back`, `dev-front` e `reviewer` (veja [Agentes](agentes.md)).

## Depois de instalar

1. Abra o **Forgejo** (`http://localhost:3000`) e entre com o usuário e a
   senha do resumo.
2. No seu repositório, **crie uma issue** descrevendo o que quer e coloque a
   label **`agent:back`** (ou `agent:front`).
3. Acompanhe: pelo painel (`./oreia`, áreas **Status** e **Uso**), pelo
   Telegram, ou na própria issue — o agente comenta e abre o pull request.
4. **Revise e aprove** o PR no Forgejo, no [px0](px0.md)
   (`http://localhost:7777`) ou pelo Telegram (`/merge 7`).

O painel é onde você vê se está tudo no ar e muda qualquer configuração —
veja [Painel](painel.md).

## Instalação automatizada

Para script, CI ou um agente de IA com acesso ao shell: sem terminal
interativo, sem perguntas. Copie
[`setup.answers.example.toml`](../setup.answers.example.toml) para
`setup.answers.toml`, preencha e rode:

```bash
./oreia install --answers setup.answers.toml
```

```toml
[llm]
url = "http://192.168.1.50:11434/v1"   # servidor compatível com /v1/chat/completions
model = "qwen2.5-coder:14b"            # modelo para planner/dev-*/executor
chat_model = ""                         # conversa direta; em branco usa o mesmo de "model"
api_key = ""                            # só se o servidor exigir

[forgejo]
owner_user = "dono"                     # seu usuário; só dá para escolher ANTES de instalar
repos = "minha-org/meu-sistema"         # dono/repo; vários separados por vírgula

[telegram]
token = ""                              # em branco = sem Telegram
allowed = ""                            # seu ID numérico (ou vários, com vírgula)

[ports]
# forgejo_http = 3000                   # só se as portas padrão estiverem ocupadas
# forgejo_ssh = 2222
# api = 3120
# px0 = 7777

[px0]
allowed_users = "dono"                  # quem pode entrar no px0

[mcp]
# enabled = true                        # liga o servidor MCP (docs/mcp.md)
# http = false                          # transporte HTTP opcional; gera MCP_TOKEN sozinho
```

Regras:

- Seção em branco ou ausente não muda o que já estava configurado.
- `llm.url` e `llm.model` andam juntos: só um dos dois é erro.
- Campo desconhecido (erro de digitação) é recusado, não ignorado.
- Nomes de usuário e repositório são validados antes de gravar qualquer coisa.

Só gravar a configuração, sem instalar: `./oreia setup-answers` (usa
`setup.answers.toml`; `ANSWERS=outro.toml` troca o arquivo). As mesmas
respostas também existem como flags do binário, para um script de uma linha:

| Flag | Equivale a |
|---|---|
| `--answers ARQUIVO.toml` | lê o arquivo primeiro; as flags abaixo sobrescrevem |
| `--llm-url`, `--llm-model`, `--llm-chat-model`, `--llm-api-key` | `[llm]` |
| `--forgejo-owner`, `--forgejo-repos` | `[forgejo].owner_user`, `[forgejo].repos` |
| `--telegram-token`, `--telegram-allowed` | `[telegram]` |
| `--px0-allowed` | `[px0].allowed_users` |
| `--mcp-enabled`, `--mcp-http` | `[mcp]` |
| `--forgejo-bootstrap` | roda `scripts/bootstrap.sh` em seguida (desnecessário com `./oreia install`) |

Para conferir o resultado sem tela: `./oreia saude` (sai com código 1 se
algum serviço configurado estiver fora do ar) e `./oreia status`.

## Comandos do dia a dia

`./oreia help` lista todos. Os principais:

| Comando | O que faz |
|---|---|
| `./oreia` | abre o painel (ou instala, se ainda não instalou) |
| `./oreia up` · `./oreia down` | sobe · para todos os serviços |
| `./oreia restart` | reinicia o orquestrador (depois de mudar a configuração fora do painel) |
| `./oreia status` | containers e checagem rápida, em texto |
| `./oreia saude` | saúde de cada serviço — Forgejo, LLM, Telegram, px0 — em texto |
| `./oreia logs [serviço]` | acompanha os logs (padrão: `oreia-seca`; ou `forgejo`, `px0`, `postgres`) |
| `./oreia doctor` | diagnóstico do ambiente, para quando algo não sobe |
| `./oreia gc` | apaga clones de tarefas do Forgejo já terminadas ([Solução de problemas](problemas.md#disco)) |
| `./oreia update` | baixa a versão nova e sobe de novo |

No Windows fora do Git Bash, troque `./oreia` por `oreia.cmd`. Quem tem
`make` pode usar `make install`, `make up`, `make painel`... — são atalhos
para os mesmos comandos.

## Atualizar

```bash
./oreia update      # git pull + reconstrói a imagem + sobe de novo
```

Seus dados (`volumes/`) e sua configuração (`.env`, `oreia-seca.toml`) não
são tocados.

## Recomeçar do zero

```bash
./oreia reset          # apaga os DADOS: Forgejo inteiro, banco, histórico. Mantém a configuração
./oreia reset --all    # apaga também .env, oreia-seca.toml e tls/ — volta ao estado de recém-clonado
./oreia install        # instala de novo
```

Os dois pedem para digitar `APAGAR` antes de fazer qualquer coisa.

O que guarda estado numa instalação:

| O quê | Para que serve |
|---|---|
| `.env` | segredos e portas: senhas, tokens, chave do LLM, token do Telegram |
| `oreia-seca.toml` | o que não é segredo: modelos, papéis, agentes, repositórios |
| `volumes/` | os dados de verdade: Forgejo (contas, repositórios, issues), tarefas e sessões do orquestrador |
| volume Docker `postgres_data` | o banco de dados do Forgejo |
| `tls/` | certificados internos usados pelo px0; o bootstrap gera de novo se faltar |
| `docker-compose.override.yml` | só existe se você ativou [containers por executor](executores.md#containers-por-executor) |

Para trocar o **nome do dono** depois de instalado é preciso um
`./oreia reset`: a conta e o token dela já foram criados com o nome antigo.

## Como funciona por baixo

Para quem quer entender (ou depurar) o que o instalador faz.

**Dois arquivos de configuração.** `oreia-seca.toml` define *o quê* (modelos,
papéis, agentes, repositórios) e nunca guarda segredo — só o *nome* de uma
variável (`api_key_env`, `owner_token_env`...). `.env` guarda os *valores*.
Nenhum dos dois vai para o git; os modelos versionados são `.env.example` e
`oreia-seca.example.toml`. Referência campo a campo em
[Configuração](configuracao.md).

**Um launcher, um binário.** `./oreia` é um script bash; `oreia.cmd` só acha
o Git Bash e repassa tudo para ele. O programa de verdade é o binário
`oreia-seca` (Rust):

- com `cargo` no PATH (quem desenvolve), o launcher roda `cargo run`;
- sem `cargo` (o normal), roda o binário pela imagem `oreia-seca:local` — a
  mesma do serviço — num container avulso com a pasta do projeto montada, de
  modo que o `.env` e o `oreia-seca.toml` lidos e gravados são os do seu disco.

**Quem fala com o Docker.** O painel só *lê* o estado dos containers
(`docker ps`/`docker stats`, pelo `docker.sock` montado nesse container
avulso). Subir, parar e o bootstrap são sempre executados pelo launcher, no
host: quando você tecla `u` no painel, ele fecha, o launcher roda
`docker compose up` e reabre o painel. Por isso as teclas funcionam igual com
ou sem `cargo`. Detalhes em [Painel](painel.md#como-as-teclas-de-ação-funcionam).

**Três caminhos, uma função.** O assistente, as áreas de configuração do
painel e o arquivo de respostas só *coletam* as respostas de jeitos
diferentes. Quem grava é sempre o mesmo código (`apply_answers` e
`toml_edits::apply`, em `crates/oreia-seca/src/setup/`) — o `.toml` é editado
preservando seus comentários, e cada gravação deixa um backup `.bak`.

**O bootstrap** (`scripts/bootstrap.sh`, chamado na etapa 3) é idempotente:
gera os segredos que faltam, os certificados de `tls/`, sobe Postgres e
Forgejo, cria o dono e uma conta + token por agente de `[[agents]]`, a conta
do MCP, a organização, os repositórios de `[forgejo].repos`, as labels, o
webhook, a proteção do `main` e o app OAuth2 do px0. Rodar de novo
(`./oreia bootstrap`) só cria o que faltar — é o que você faz depois de
acrescentar um agente ou um repositório.

**Portas.** `FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT`,
`PX0_PORT` e `MCP_HTTP_PORT` no `.env` só mudam a porta que você acessa *de
fora*. A comunicação entre os containers (webhook, `[forgejo].url`) usa a
rede interna do Docker e não muda.
