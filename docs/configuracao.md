# Configuração

Dois arquivos: `oreia-seca.toml` (o que fazer) e `.env` (segredos). O `oreia-seca.toml`
nunca guarda segredos, só o **nome** da variável (`*_env`). Campos
desconhecidos geram erro, então um erro de digitação não passa despercebido.

Preenchendo do zero? `make setup` (terminal), `make setup-web` (página local)
ou `make setup-answers` (arquivo/flags, para automatizar ou deixar um agente
de IA fazer) cobrem Telegram, LLM e o bootstrap do Forgejo numa coisa só —
veja [Instalação](instalacao.md). O resto deste documento é a referência de
cada campo, para quando quiser editar à mão.

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
| `[models.<nome>]` | cada modelo: `base_url` **ou** `base_url_env`, `model` **ou** `model_env`, `context`, `reserve_output`, `api_key_env`, `timeout_secs`, `temperature` |
| `[roles]` | papel → lista de modelos (titular primeiro) |
| `[forgejo]` | URL, segredo do webhook, seu token, repositórios, branch base, estilo de merge |
| `[[agents]]` | um bloco por agente |

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
`make llm-setup` preenche essas variáveis interativamente; veja
[Modelos locais](modelos-locais.md).

## .env

| Variável | Quem preenche |
|---|---|
| `TELEGRAM_TOKEN`, `TELEGRAM_ALLOWED` | você |
| `REST_API_KEY`, `FORGEJO_WEBHOOK_SECRET`, `FORGEJO_DB_PASSWORD`, `FORGEJO_OWNER_PASSWORD` | `make bootstrap` gera |
| `FORGEJO_OWNER_TOKEN`, `AGENT_*_TOKEN` | `make bootstrap` gera |
| `LLM_CODER_URL/MODEL`, `LLM_CHAT_URL/MODEL`, `LLM_API_KEY` | `make llm-setup` gera |
| `FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT` | você, só se 3000/2222/3120 já estiverem ocupadas nessa máquina (padrão em `.env.example`) |
| `OPENROUTER_KEY`, `GEMINI_KEY` | você, se usar modelos remotos específicos (exemplos comentados no `.toml`) |

## API REST

```bash
curl -X POST http://localhost:3120/api/message \
  -H "Authorization: Bearer $REST_API_KEY" -H "Content-Type: application/json" \
  -d '{"text": "gere o relatório semanal de PRs", "userId": "cron-semanal", "source": "cron"}'
```

Resposta `202` imediata; o plano chega no Telegram com botões de aprovar/rejeitar.
`GET /api/status` devolve fila, aprovações pendentes, papéis e agentes.
