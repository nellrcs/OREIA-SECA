# Modelos locais

O OREIA-SECA fala com qualquer servidor que implemente `POST /v1/chat/completions`
(provider `openai_compat`, o padrão) e também com a API própria da Anthropic
(provider `anthropic`, veja [Anthropic](#anthropic) abaixo). Para o caso
compatível com OpenAI não há código específico por provedor: muda só o
`base_url`. O `docker-compose.yml` não sobe nenhum LLM junto — o orquestrador
sempre espera um servidor externo (outra máquina, seu computador, ou uma
API), configurado com:

```bash
make llm-setup
```

Esse assistente pergunta a URL do servidor, lista os modelos disponíveis e
grava `LLM_CODER_URL`/`LLM_CODER_MODEL` (planner, dev-*, executor) e
`LLM_CHAT_URL`/`LLM_CHAT_MODEL` (conversa direta) no `.env` — o
`oreia-seca.example.toml` já referencia essas variáveis via `base_url_env`/
`model_env`. Também dá para rodar sem perguntas: `oreia-seca setup llm --url
http://<ip>:11434/v1 --model qwen2.5-coder:14b`.

| Servidor | URL |
|---|---|
| Ollama em outra máquina | `http://<ip>:11434/v1` |
| LM Studio no seu computador | `http://host.docker.internal:1234/v1` |
| llama.cpp (`llama-server`) | `http://<host>:8080/v1` |
| vLLM | `http://<host>:8000/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` + chave |
| Gemini (endpoint compatível) | `https://generativelanguage.googleapis.com/v1beta/openai` + chave |

## Rodando o modelo em outra máquina

A forma recomendada: o LLM roda numa máquina com GPU, o OREIA-SECA roda em
outra (ou num LXC leve), e os dois se falam pela rede local.

**Ollama**

1. Na máquina com a GPU, faça o Ollama escutar na rede, não só em
   `localhost`: `OLLAMA_HOST=0.0.0.0 ollama serve` (ou, se for o serviço do
   systemd, adicione `Environment="OLLAMA_HOST=0.0.0.0"` e reinicie).
2. Libere a porta 11434 no firewall para a sua rede local.
3. Teste de outra máquina: `curl http://<ip>:11434/v1/models`.
4. Rode `make llm-setup` apontando para `http://<ip>:11434/v1`.

**LM Studio**

1. Carregue o modelo, abra a aba do servidor e ligue "Serve on Local Network"
   (não só `localhost`).
2. Teste com `curl http://<ip>:1234/v1/models` de outra máquina.
3. `make llm-setup` apontando para `http://<ip>:1234/v1`.

> ⚠️ **Segurança.** Nem o Ollama nem o LM Studio exigem autenticação por
> padrão. Exponha a porta só na sua rede local (nunca direto na internet) ou
> por VPN (Tailscale, WireGuard). Se for uma API com chave (OpenRouter,
> Gemini), o assistente pergunta a chave e ela nunca aparece em `/models` nem
> no `--check`.

Prefere rodar o modelo na mesma máquina do OREIA-SECA? Instale o Ollama ou o
LM Studio normalmente nela (fora do compose deste projeto) e use
`http://host.docker.internal:11434/v1` — é o endereço que o container do
orquestrador usa para chegar num servidor no próprio host, já liberado em
`docker-compose.yml`. `make llm-setup --url http://host.docker.internal:11434/v1
--model <modelo>` grava isso direto; o passo de listar modelos pode falhar
rodando assim (esse endereço só resolve de dentro do container), mas não
impede o resto — digite o nome do modelo manualmente.

## O que considerar ao escolher

- **Siga bem instruções de formato.** O executor depende do modelo escrever `<action>` corretamente. Modelos "coder" costumam ir melhor. O parser é tolerante (aceita vários formatos), mas um modelo que ignora o formato não consegue agir.
- **Janela de contexto real.** Configure `context` com o valor que o servidor realmente usa. No Ollama o contexto padrão é pequeno; aumente com `num_ctx` no Modelfile ou na variável `OLLAMA_CONTEXT_LENGTH`, senão o modelo "esquece" o começo da fase.
- **VRAM.** Como regra grosseira: modelos de ~7–8B rodam em 8 GB; ~14B pedem ~12–16 GB; ~32B pedem ~24 GB (quantizados em 4 bits). Confira o catálogo do Ollama para as versões atuais.

Rodando o orquestrador numa máquina separada e pequena (sem GPU, ex. i5 + 8 GB
de RAM), enquanto o LLM fica na máquina com GPU como descrito acima? Veja
[Benchmark e hardware recomendado](benchmark.md) para orçamento de memória e
um script (`scripts/benchmark.sh`) que mede o uso real na sua máquina.

## Anthropic

Para usar Claude com uma chave direto do console.anthropic.com (em vez de via
OpenRouter), use `provider = "anthropic"` em `[models.*]` — a API da Anthropic
(`/v1/messages`) não é compatível com `/v1/chat/completions`, então ela tem um
provedor próprio (`AnthropicNative`, em `oreia-llm`), não o `base_url` genérico:

```toml
[models.claude]
provider = "anthropic"
base_url = "https://api.anthropic.com/v1"
model = "claude-sonnet-5-5"
api_key_env = "ANTHROPIC_API_KEY"   # obrigatório para este provider — nunca vazio
context = 200000
reserve_output = 8192
```

`ANTHROPIC_API_KEY` vai no `.env`, nunca no `.toml`. Diferente do
`openai_compat`, aqui `api_key_env` é obrigatório: sem uma chave válida a API
da Anthropic rejeita a chamada, então o boot já marca o modelo como "não
configurado" se a variável estiver ausente ou vazia.

Depois de declarado em `[models]`, `claude` funciona como qualquer outro
modelo: entra num `[roles]` (ex.: `planner = ["claude", "coder"]`) ou como
mais um degrau em `[escalation.<papel>]` — útil para escalar de um modelo
mais barato para o Claude quando o primeiro degrau falhar a verificação (veja
[Executores e escalada](executores.md)).

## Depurando uma chamada

Esses provedores (`openai_compat` e `anthropic`) ainda são experimentais —
quando uma tarefa falha de um jeito estranho (resposta vazia, formato
inesperado, escalada disparando sem motivo claro), o jeito mais rápido de
entender o que aconteceu é ver exatamente o que foi enviado e o que o modelo
respondeu. Ligue com:

```bash
OREIA_SECA_LLM_DEBUG=1
```

no `.env` (lido pelo `env_file: .env` do `docker-compose.yml`, sem precisar
tocar no compose). Com isso, cada chamada loga em `debug` o corpo completo da
requisição e da resposta crua — a chave da API nunca aparece, porque ela vai
no cabeçalho HTTP, não no corpo. Combine com `RUST_LOG=oreia_llm=debug` (veja
[Segurança](seguranca.md)) para esses logs aparecerem — `RUST_LOG=info`
sozinho não mostra `debug!`. Desligado por padrão porque o conteúdo logado
inclui prompts e código do usuário; não deixe ativado em produção.

## Estratégia híbrida

O planejamento é a etapa que mais se beneficia de um modelo forte, e é a que menos gasta tokens (uma chamada por tarefa). A execução gasta muito mais e pode ficar num modelo menor.

```toml
[roles]
planner  = ["planner-remoto", "coder"]   # remoto, com o local/externo de reserva
dev-back = ["coder"]
direct   = ["chat"]
```

Se o remoto cair ou estourar o tempo, o roteador usa o próximo da lista automaticamente. Se nenhum modelo de um papel estiver acessível, o OREIA-SECA ainda sobe normalmente — `/models` e `/api/status` mostram o estado de cada um, e uma tarefa que precisar desse papel recebe um aviso em vez de travar.
