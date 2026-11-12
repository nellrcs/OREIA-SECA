# Configuração

Dois arquivos: `oreia-seca.toml` (o que fazer) e `.env` (segredos). O `oreia-seca.toml`
nunca guarda segredos, só o **nome** da variável (`*_env`). Campos
desconhecidos geram erro, então um erro de digitação não passa despercebido.

Você raramente precisa abrir esses arquivos: `./oreia install` cria os dois e
pergunta o essencial, e o painel (`./oreia`) edita px0, Forgejo, MCP,
Telegram, LLM, Executor e Servidor — veja [Instalação](instalacao.md) e
[Painel](painel.md#configurar). Esta página é a referência campo a campo,
para o que o painel ainda não cobre (agentes, papéis, escalada, modelos
extras) e para quem prefere editar na mão. Para recomeçar com arquivos
limpos, veja [Recomeçar do zero](instalacao.md#recomeçar-do-zero).

Valide com:

```bash
make check     # ou: oreia-seca --check --config oreia-seca.toml
```

## oreia-seca.toml

Veja [`oreia-seca.example.toml`](../oreia-seca.example.toml), que está comentado linha a linha. Resumo:

| Seção | Para quê |
|---|---|
| `[server]` | porta da API e variável do token Bearer |
| `[paths]` | onde ficam dados, workspaces e skills |
| `[queue]` | concorrência e timeout por tarefa |
| `[approval]` | quais fontes esperam `/approve` (padrão: `rest`, `cron`) |
| `[telegram]` · `[terminal]` | canais humanos |
| `[llm]` | cooldown após falha, timeout padrão e cadeia de reserva global |
| `[executor]` | passos por fase, tentativas, timeout de shell, máximo de fases, executor padrão |
| `[executors.*]` · `[escalation.*]` | executores externos e escalada por papel — veja [Executores e escalada](executores.md) |
| `[models.<nome>]` | cada modelo: `provider` (`"openai_compat"` padrão ou `"anthropic"`), `base_url` **ou** `base_url_env`, `model` **ou** `model_env`, `context`, `reserve_output`, `api_key_env`, `timeout_secs`, `temperature` |
| `[roles]` | papel → lista de modelos (titular primeiro) |
| `[forgejo]` | URL, segredo do webhook, seu token, repositórios, branch base, estilo de merge |
| `[[agents]]` | um bloco por agente |
| `[mcp]` | servidor MCP (P.O. por IA + desenvolvedor sob demanda), opcional — veja [Servidor MCP](mcp.md) |

### Papéis

| Papel | Usado em |
|---|---|
| `planner` | quebrar o pedido em fases (JSON) |
| `executor` | executar fases de tarefas que não vieram do Forgejo |
| `direct` | conversa direta |
| `<nome do agente>` | executar fases das issues daquele agente (ou o `model_role` do agente) |
| `default` | qualquer papel sem entrada própria |

Ordem de tentativa: modelos do papel → modelos de `default` → `[llm].fallback`.
Um modelo que cai ou estoura o tempo fica fora por `cooldown_secs` e volta sozinho.
Um modelo cuja `base_url_env`/`model_env` não está definida fica "não configurado"
(não derruba o boot) — `/models`, `GET /api/status` e `--check` mostram qual falta.
O assistente de instalação e a área **LLM** do painel preenchem essas
variáveis (ou `./oreia llm-setup`, só por perguntas no terminal); a área
**Saúde** mostra se o servidor responde e se o modelo existe nele. Veja
[Modelos locais](modelos-locais.md).

### Servidor, Diretórios e Fila (`[server]`, `[paths]`, `[queue]`)

São as configurações centrais do orquestrador (editáveis também pela opção "Servidor" na TUI de setup):

```toml
[server]
host = "0.0.0.0"                  # IP onde a API REST escuta
port = 3120                       # porta da API HTTP (OREIA_SECA_PORT no .env)
api_key_env = "REST_API_KEY"      # segredo Bearer para autenticação interna

[paths]
data = "data"                     # histórico de tarefas (data/tasks/), sessões e claims
workspace = "workspace"           # diretório de clonagem para agentes da via automática
skills = "skills"                 # scripts de skills disponíveis para o executor

[queue]
concurrency = 1                   # tarefas paralelas (1 evita disputa por GPU em modelos locais)
timeout_secs = 1800               # limite de 30 minutos antes de abortar tarefa travada
```

### Servidor MCP (`[mcp]`)

Configura a integração com clientes MCP (Claude Code, Claude Desktop, Cursor):

```toml
[mcp]
enabled = true                    # habilita a via MCP (padrão: false / comentado)
agent_user = "agent-mcp"          # usuário do Forgejo criado pelo bootstrap
agent_token_env = "AGENT_MCP_TOKEN" # variável com o token da conta agent-mcp
workdir = "workspace/mcp"         # diretório persistente para clonagem das issues do MCP
max_review_rounds = 3             # limite de ciclos de request_changes antes de exigir decisão humana
http = false                      # opcional: habilita transporte HTTP além do stdio padrão
bind = "127.0.0.1:3121"           # endereço do servidor HTTP MCP (exige MCP_TOKEN no .env)
```


## .env

| Variável | Quem preenche |
|---|---|
| `TELEGRAM_TOKEN`, `TELEGRAM_ALLOWED` | você |
| `REST_API_KEY`, `FORGEJO_WEBHOOK_SECRET`, `FORGEJO_DB_PASSWORD`, `FORGEJO_OWNER_PASSWORD` | o bootstrap (`./oreia install`) gera |
| `FORGEJO_OWNER_TOKEN`, `AGENT_*_TOKEN`, `AGENT_MCP_TOKEN` | o bootstrap (`./oreia install`) gera |
| `MCP_TOKEN` | você, só se usar o transporte HTTP opcional do MCP (`[mcp].http = true`) — veja [Servidor MCP](mcp.md) |
| `FORGEJO_REPOS` | repositórios monitorados (gerado/mantido pelo `bootstrap` a partir do `.toml`) |
| `PX0_PORT`, `PX0_ALLOWED_USERS`, `PX0_OAUTH_CLIENT_ID`, `PX0_OAUTH_CLIENT_SECRET` | interface web px0 (`bootstrap` gera credenciais OAuth2) |
| `LLM_CODER_URL/MODEL`, `LLM_CHAT_URL/MODEL`, `LLM_API_KEY` | assistente / área LLM do painel |
| `FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT` | você, só se 3000/2222/3120 já estiverem ocupadas nessa máquina (padrão em `.env.example`) |
| `OPENROUTER_KEY`, `GEMINI_KEY`, `ANTHROPIC_API_KEY` | você, se usar modelos remotos específicos (exemplos comentados no `.toml`) |
| `DOCKER_GID`, `OREIA_SECA_HOST_DIR` | você, se usar executores externos rodando em container separado (veja [Executores](executores.md)) |
| `OREIA_SECA_LLM_DEBUG` | você, opcional (`1` para depurar prompts/respostas de LLM nos logs) |

## Checagem ao salvar

Toda vez que a configuração muda — por `--check --live` ou pelo `/check` do Telegram —, o
`oreia-seca` pode checar de verdade se o que mudou
funciona, não só se o TOML tem a forma certa. São checagens curtas (poucos segundos cada, não um
benchmark): resolvem as variáveis de ambiente, batem num `GET /models` de cada modelo tocado e
conferem se o modelo configurado está na lista, tentam uma chamada mínima de geração (pulada para
modelo remoto com `api_key_env` — custo), confirmam que o binário (ou a imagem Docker) de cada
executor existe, que o Forgejo responde e que cada token aponta pro usuário certo, e por aí vai —
veja a tabela em `crates/oreia-seca/src/check/mod.rs`.

Erro de rede (conexão recusada, timeout) nunca bloqueia — vira aviso ⚠️, porque "a máquina pode
estar desligada" não é motivo pra recusar salvar. Só bloqueiam (❌): TOML/forma inválida, modelo
que não existe no servidor, token rejeitado (401/403), e binário/imagem de executor ausente.

```bash
# tudo (sem comparar com nada)
oreia-seca --check --live --config oreia-seca.toml

# só o que mudou desde um TOML antigo
oreia-seca --check --live --since oreia-seca.toml.bak --config oreia-seca.toml
```

A API expõe o mesmo mecanismo em `POST /api/check` (Bearer, igual o resto de `/api`):

```bash
curl -X POST http://localhost:3120/api/check \
  -H "Authorization: Bearer $REST_API_KEY" -H "Content-Type: application/json" \
  -d "{\"candidate\": \"$(cat oreia-seca.toml | sed 's/\"/\\\\\"/g' | tr '\n' ' ')\"}"
```

Ele faz o diff contra a config ATUAL em memória (nunca a substitui) e devolve
`{"results": [{"id", "target", "status", "message", "took_ms"}, ...]}`. Pelo Telegram, `/check`
roda tudo e resume (os ✅ só são contados; ⚠️ e ❌ aparecem por extenso). Todo resultado é gravado em
`data/checks.jsonl` (mesmo espírito de `data/escalation.jsonl`).

O [painel](painel.md) não chama essa checagem antes de salvar — grava o `.toml`/`.env` e oferece
reiniciar o orquestrador. O que ele tem no lugar é a área **Saúde**, que confere continuamente os
serviços (Forgejo, LLM, Telegram, px0) com a configuração que está valendo; `--check --live` continua
sendo o teste mais fundo (executores, tokens de cada agente, escalada).

## API REST

```bash
curl -X POST http://localhost:3120/api/message \
  -H "Authorization: Bearer $REST_API_KEY" -H "Content-Type: application/json" \
  -d '{"text": "gere o relatório semanal de PRs", "userId": "cron-semanal", "source": "cron"}'
```

Resposta `202` imediata; o plano chega no Telegram com botões de aprovar/rejeitar.
`GET /api/status` devolve fila, aprovações pendentes, papéis e agentes.

### Endpoints Internos do MCP (`/api/mcp/*`)

Usados pelo crate `oreia-mcp` (com o mesmo Bearer `REST_API_KEY`) para despachar as ferramentas do protocolo MCP:
* `POST /api/mcp/create_story`: cria uma issue com template estruturado e critérios de aceite.
* `POST /api/mcp/list_stories`: lista histórias por status (`available`, `in_progress`, `done`).
* `POST /api/mcp/get_story`: consulta texto, tarefas e histórico de falhas de uma issue.
* `POST /api/mcp/get_pr`: inspeciona título, autor, diff e status de um PR.
* `POST /api/mcp/checkout_issue`: registra a posse persistente em `data/claims.json` e prepara o workspace em `workspace/mcp/`.
* `POST /api/mcp/run_checks`: executa a verificação/testes do projeto na pasta do checkout.
* `POST /api/mcp/submit_issue`: com os checks verdes, faz commit e push com `agent-mcp` e abre/atualiza o PR.
* `POST /api/mcp/release_issue`: remove a posse e opcionalmente devolve a issue ao agente original.
* `POST /api/mcp/review_pr`: adiciona comentário ou solicita alterações no PR (`request_changes`).
* `POST /api/mcp/dispatch_issue`: transfere a issue para um agente com opção de pular degraus de escalada.
* `POST /api/mcp/stats`: devolve estatísticas consolidadas de resolução via MCP e custos.

