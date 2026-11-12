# Servidor MCP (P.O. por IA + desenvolvedor sob demanda)

O OREIA tem duas vias de trabalho, que nunca disputam a mesma issue ao mesmo tempo:

| Via | Como aciona | Quem decide o quê fazer |
|---|---|---|
| **Automática** (já existia) | label `agent:*` numa issue | o OREIA escolhe o agente/modelo pela `[escalation.<papel>]` configurada |
| **MCP** (este documento) | um cliente MCP (Claude Code, Claude Desktop...) chama uma ferramenta | você, pela conversa com o cliente — ele assume a issue e codifica com o **modelo dele próprio** |

Na via MCP, o OREIA cuida só de git, checks e PR — nunca do modelo: quem "pensa" é o
cliente que está chamando as ferramentas. É útil para duas coisas:

- **P.O.**: escrever histórias (`create_story`), acompanhar o que está livre/em
  andamento/concluído (`list_stories`/`get_story`), revisar PRs
  (`get_pr`/`review_pr`) — tudo pela conversa, sem abrir o Forgejo.
- **Desenvolvedor sob demanda**: pegar uma issue livre, ou uma que um agente
  tentou e não conseguiu, e resolver com o modelo do seu próprio cliente
  (`checkout_issue` → `run_checks` → `submit_issue`).

## Regra de posse

Uma issue (ou PR) só tem um dono por vez: um agente (via automática) **ou** o MCP
(`claimed:mcp`), nunca os dois. `checkout_issue` recusa se já houver dono; o webhook
da via automática ignora qualquer issue com a label `claimed:mcp`/`agent:mcp` ou já
reivindicada pelo MCP. A posse é persistente (`data/claims.json`): sobrevive a um
restart do `oreia-seca` — só `release_issue` (ou o merge do PR) libera.

## Registrar no Claude Code

### Opção 1: Binário local (ou via ./oreia mcp)
```bash
claude mcp add oreia-seca -- ./oreia mcp
# ou direto com o binário compilado:
claude mcp add oreia-seca -- target/release/oreia-seca mcp --config oreia-seca.toml
```

### Opção 2: Container Docker em execução
Se o OREIA estiver rodando dentro do Docker (`./oreia up`):
```bash
claude mcp add oreia-seca -- docker compose exec -T oreia-seca oreia-seca mcp --config /app/oreia-seca.toml
```

## Registrar no Claude Desktop

Edite `claude_desktop_config.json` (Settings → Developer → Edit Config):

### Modo Stdio (Local ou Docker)
```json
{
  "mcpServers": {
    "oreia-seca": {
      "command": "docker",
      "args": ["compose", "exec", "-T", "oreia-seca", "oreia-seca", "mcp", "--config", "/app/oreia-seca.toml"]
    }
  }
}
```
*(Se estiver rodando fora do Docker, substitua `"command"` pelo caminho do binário local `oreia-seca` e `"args": ["mcp", "--config", "oreia-seca.toml"]`).*

### Modo HTTP Streamable (Cursor, Claude Desktop ou Máquina Remota)
Com `[mcp].http = true`, `MCP_TOKEN` no `.env` e — se estiver rodando via
`docker compose` — `[mcp].bind = "0.0.0.0:3121"` (ver
[Transporte HTTP](#transporte-http-opcional) abaixo), falta ainda **ligar o
processo que escuta essa porta** — ao contrário do modo stdio acima (onde o
próprio cliente sobe um `oreia-seca mcp` novo a cada conexão), o HTTP precisa
de um processo rodando de forma contínua, independente de qualquer cliente
conectado:
```bash
./oreia mcp-http    # ou: make mcp-http
```
Isso roda `oreia-seca mcp --config /app/oreia-seca.toml` em segundo plano,
dentro do container `oreia-seca` já em execução (`docker compose exec -d`) —
não é um serviço do `docker-compose.yml`, então **não sobrevive a um restart
do container**: rode de novo depois de `./oreia up`/`docker compose restart
oreia-seca`. Depois, configure o cliente:
```json
{
  "mcpServers": {
    "oreia-seca": {
      "url": "http://127.0.0.1:3121/mcp",
      "headers": {
        "Authorization": "Bearer SEU_MCP_TOKEN_AQUI"
      }
    }
  }
}
```

Reinicie o Claude Desktop (ou recarregue o cliente) depois de salvar.


## Pré-requisito: `[mcp]` configurado

```toml
# oreia-seca.toml
[mcp]
enabled = true
agent_user = "agent-mcp"          # criado pelo bootstrap (./oreia install ou ./oreia bootstrap)
agent_token_env = "AGENT_MCP_TOKEN"
workdir = "workspace/mcp"
max_review_rounds = 3
```

Sem `[mcp]` no config (ou sem rodar `oreia-seca mcp`), nada muda: é exatamente o
comportamento de antes desta feature existir. Em vez de descomentar o bloco à
mão, a área **MCP** do painel (`./oreia`; ou `--mcp-enabled`/`--mcp-http` no
caminho não interativo) também liga/desliga `[mcp]` e o transporte HTTP
opcional — veja [Painel](painel.md#configurar). A área **Saúde** mostra se a
porta do transporte HTTP está aberta.

## Diretório de trabalho

`checkout_issue` clona em `{[mcp].workdir}/<dono>-<repo>-<issue>` — diferente de
`[paths].workspace` (que a via automática apaga a cada rodada), este diretório
**persiste** entre chamadas (`checkout_issue` → `run_checks` → `submit_issue`), porque
é nele que o modelo do seu cliente vai ler e escrever código.

O cliente MCP (o processo que roda o modelo) precisa **enxergar esse caminho** —
funciona de graça quando ele roda na mesma máquina que o `oreia-seca`. Com o
`oreia-seca` num servidor remoto, as opções são: montar `workspace/mcp` como volume
acessível pelo cliente, ou (fase futura, fora de escopo desta versão) clonar
localmente e usar `submit_issue` só pra publicar o branch já pronto.

## Ferramentas

| Ferramenta | O que faz |
|---|---|
| `create_story` | cria uma issue com template fixo (Objetivo, Critérios de aceite, Arquivos prováveis, Fora de escopo) — recusa sem pelo menos 1 critério de aceite |
| `list_stories` | lista issues por status: `available` (livre, agente falhou, ou PR esperando ajuste), `in_progress`, `done` |
| `get_story` | issue completa: texto, dono atual, fases da última tentativa, motivo da falha (se houver), PR |
| `get_pr` | título, autor, arquivos alterados, diff (truncado), notas de revisão, escalada |
| `review_pr` | `comment` só comenta; `request_changes` aciona o agente dono do branch (como `/changes`), limitado a `[mcp].max_review_rounds` rodadas por PR. Nunca aprova nem faz merge — isso é sempre seu, no Forgejo ou via `/merge` |
| `checkout_issue` | assume a issue: toma a posse, prepara o branch certo (continua de um agente que falhou, se for o caso) e devolve o diretório de trabalho, os critérios de aceite e o motivo da falha anterior |
| `run_checks` | roda a mesma verificação do executor automático (`test_cmd` do papel que tentou antes, se houver) |
| `submit_issue` | só com os checks verdes: commit + push com a conta `agent-mcp`, abre/atualiza o PR, comenta na issue |
| `release_issue` | libera a posse; com `return_to_agent`, devolve a issue pro agente original tentar de novo |
| `dispatch_issue` | atalho pra via automática — como `/assign`, com a opção de forçar um degrau específico da escalada |
| `get_stats` | o mesmo do `/stats` |

## Fluxo de exemplo

```
Você: crie uma história pra corrigir o cálculo de frete com cupom
Claude (create_story): criada acme/app#42, sem agente — livre pra análise

Você: manda pro dev-back
Claude (dispatch_issue repo=acme/app n=42 agent=dev-back): despachado

[dev-back tenta, falha duas vezes — veja /stats ou list_stories status=available]

Você: assume você mesmo e conserta
Claude (checkout_issue repo=acme/app n=42):
  → workdir: workspace/mcp/acme-app-42
  → motivo da falha anterior: "testes do frete não cobriam cupom + frete grátis"
  [Claude lê o código, corrige, roda a suíte localmente]
Claude (run_checks repo=acme/app n=42): passou
Claude (submit_issue repo=acme/app n=42 summary="..."): PR aberto, acme/app#43
```

`/stats` (ou `get_stats`) passa a mostrar "resolvidas via MCP após falha: 1 (dev-back: 1)".

## Prompt `po_guidelines`

Um prompt MCP com as regras pra escrever uma boa história: pequena (uma por
agente — se front e back dependem um do outro, duas histórias com o contrato
definido primeiro), critérios verificáveis de fora, objetivo antes de "como fazer".

## Recursos (somente leitura)

- `oreia://agents/<nome>` — o `AGENT.md` daquele agente.
- `oreia://docs/roadmap` — `docs/roadmap.md`.
- `oreia://repos/<repo>/contract` — ainda sem fonte configurada (recusa com mensagem clara).

## Transporte HTTP (opcional)

Por padrão o MCP fala só stdio (subprocesso local). Pra um cliente que não roda na
mesma máquina, `[mcp].http = true` + `MCP_TOKEN` no `.env` expõe `POST /mcp` em
`[mcp].bind` (padrão `127.0.0.1:3121`) — Bearer `MCP_TOKEN` e uma allowlist simples
de `Origin` (hosts locais). Sem `MCP_TOKEN` definido, o transporte HTTP nem sobe.
Os dois transportes coexistem; o HTTP nunca substitui o stdio.

**Rodando via `docker compose` (o padrão deste projeto):** o `127.0.0.1` do
`bind` padrão é o loopback de DENTRO do container do `oreia-seca`, não o seu —
mesmo com a porta publicada no `docker-compose.yml` (`MCP_HTTP_PORT`, padrão
3121), nada chega lá. Use `[mcp].bind = "0.0.0.0:3121"` nesse caso (mesmo
nível de exposição que a API REST já tem na porta 3120 — protegido pelo
Bearer `MCP_TOKEN`, não por estar só em loopback). Depois de mudar o
`oreia-seca.toml`, reinicie o container (`docker compose restart oreia-seca`)
pra aplicar.

## Segurança

Veja [Segurança](seguranca.md#servidor-mcp) — resumo: nenhuma ferramenta devolve
token/segredo, commit e push só pelo `gitops` (mesma garantia dos agentes da via
automática), `submit_issue` nunca empurra pra branch base, e toda ferramenta que
escreve registra em `data/mcp.jsonl` (ferramenta, repo, issue, resultado — nunca
conteúdo sensível).
