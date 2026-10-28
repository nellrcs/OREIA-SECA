# Configuração

Dois arquivos: `oreia-seca.toml` (o que fazer) e `.env` (segredos). O `oreia-seca.toml`
nunca guarda segredos, só o **nome** da variável (`*_env`). Campos
desconhecidos geram erro, então um erro de digitação não passa despercebido.

Preenchendo do zero? `make setup` (terminal) ou `make setup-answers`
(arquivo/flags, para automatizar ou deixar um agente de IA fazer) cobrem
Telegram, LLM e o bootstrap do Forgejo numa coisa só —
veja [Instalação](instalacao.md). Reinstalando depois de já ter usado antes
(ou com `.env`/`oreia-seca.toml` suspeitos de estarem com lixo de uma
configuração anterior)? Veja [Começar do zero](instalacao.md#começar-do-zero-reinstalação-limpa).
O resto deste documento é a referência de cada campo, para quando quiser
editar à mão.

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
| `FORGEJO_REPOS` | repositórios monitorados (gerado/mantido pelo `bootstrap` a partir do `.toml`) |
| `PX0_PORT`, `PX0_ALLOWED_USERS`, `PX0_OAUTH_CLIENT_ID`, `PX0_OAUTH_CLIENT_SECRET` | interface web px0 (`bootstrap` gera credenciais OAuth2) |
| `LLM_CODER_URL/MODEL`, `LLM_CHAT_URL/MODEL`, `LLM_API_KEY` | `make llm-setup` gera |
| `FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`, `OREIA_SECA_PORT` | você, só se 3000/2222/3120 já estiverem ocupadas nessa máquina (padrão em `.env.example`) |
| `OPENROUTER_KEY`, `GEMINI_KEY`, `ANTHROPIC_API_KEY` | você, se usar modelos remotos específicos (exemplos comentados no `.toml`) |
| `DOCKER_GID`, `OREIA_SECA_HOST_DIR` | você, se usar executores externos rodando em container separado (veja [Executores](executores.md)) |
| `OREIA_SECA_LLM_DEBUG` | você, opcional (`1` para depurar prompts/respostas de LLM nos logs) |

## Checagem ao salvar

Toda vez que a configuração muda — pelo painel de Configuração do [px0](px0.md), por `--check
--live` ou pelo `/check` do Telegram —, o `oreia-seca` pode checar de verdade se o que mudou
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
`{"results": [{"id", "target", "status", "message", "took_ms"}, ...]}`. O painel de
Configuração do px0 chama isso antes de gravar `oreia-seca.toml`: com algum `fail`, nada é salvo;
só com `warn`, salva e mostra os avisos (veja [px0 → Configuração](px0.md#configuração-config--opt-in-exige-dockersock)).
Pelo Telegram, `/check` roda tudo e resume (os ✅ só são contados; ⚠️ e ❌ aparecem por extenso).
Todo resultado é gravado em `data/checks.jsonl` (mesmo espírito de `data/escalation.jsonl`).

## API REST

```bash
curl -X POST http://localhost:3120/api/message \
  -H "Authorization: Bearer $REST_API_KEY" -H "Content-Type: application/json" \
  -d '{"text": "gere o relatório semanal de PRs", "userId": "cron-semanal", "source": "cron"}'
```

Resposta `202` imediata; o plano chega no Telegram com botões de aprovar/rejeitar.
`GET /api/status` devolve fila, aprovações pendentes, papéis e agentes.
