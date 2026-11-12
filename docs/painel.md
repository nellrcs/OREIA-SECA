# Painel

```bash
./oreia            # no Windows, fora do Git Bash: oreia.cmd
```

Uma tela no terminal para ver se está tudo no ar, quanto os agentes estão
consumindo, e mudar qualquer configuração — sem abrir `.env` nem
`oreia-seca.toml`.

```
 OREIA-SECA   ● 6/7 serviços no ar  ·  fila: 1 rodando, 2 na espera  ·  atualizado 14:03:11
┌──────────────────┐┌ Acesso ──────────────────────────────────────────────────────────────┐
│PAINEL            ││ Forgejo (git)   http://localhost:3000     ● no ar · 12 ms            │
│ ● Status         ││ px0 (revisão)   http://localhost:7777     ● no ar · 8 ms             │
│ ♥ Saúde          ││ API             http://localhost:3120     ● no ar · 4 ms             │
│ ▤ Uso            ││ Fila            1 rodando · 2 na espera · 14 concluída(s) · 1 falha(s)│
│                  ││ Em andamento    acme/loja#12                                         │
│CONFIGURAR        │└──────────────────────────────────────────────────────────────────────┘
│ ✓ px0            │┌ Containers ──────────────────────────────────────────────────────────┐
│ ✓ Forgejo        ││Serviço     Estado                 CPU   CPU (histórico)  Memória     │
│ ○ MCP            ││oreia-seca  Up 2 hours (healthy)   3.0%  ▂▄▃▆▃▂▂▃▅▂       48MiB / 15GiB│
│ ● Telegram       ││forgejo     Up 2 hours (healthy)   6.0%  ▂▅▃█▄▃▂▂▃▂       212MiB / …  │
│ ● LLM            ││postgres    Up 2 hours (healthy)   0.3%  ▁▁▁▁▁▁▁▁▁▁       61MiB / …   │
│ ● Executor       │└──────────────────────────────────────────────────────────────────────┘
│ ● Servidor       │┌ Checklist ───────────────────────────────────────────────────────────┐
└──────────────────┘│ ✓ Bootstrap do Forgejo: contas e tokens de acesso já gerados         │
```

À esquerda, dois grupos: **Painel** (o que você olha — só leitura, atualiza
sozinho) e **Configurar** (o que você edita). A linha do topo resume o
essencial e aparece em qualquer área.

- [Teclas](#teclas)
- [Status](#status)
- [Saúde](#saúde)
- [Uso](#uso)
- [Configurar](#configurar)
- [Sem tela: `saude` e `status`](#sem-tela-saude-e-status)
- [Como as teclas de ação funcionam](#como-as-teclas-de-ação-funcionam)

## Teclas

| Onde | Tecla | O que faz |
|---|---|---|
| Lista de áreas | `↑` `↓` (ou `k` `j`) | troca de área |
| | `Enter` / `Tab` | entra nos campos de uma área de configuração |
| | `Ctrl+S` / `F2` | resumo do que mudou e salvar |
| | `q` / `Esc` | sai (sem salvar) |
| Status, Saúde, Uso | `r` | atualiza agora, sem esperar o intervalo |
| | `u` · `d` | **sobe** · **para** todos os serviços |
| | `i` | reinicia o orquestrador |
| | `b` | roda o bootstrap do Forgejo |
| | `l` | mostra os logs do orquestrador (`Ctrl+C` volta ao painel) |
| | `a` | abre o assistente de instalação (as 4 perguntas) |
| Dentro de um campo | digitar / `Backspace` | edita |
| | `Espaço` | liga/desliga, ou troca o preset |
| | `Tab` / `Enter` / `↓` · `Shift+Tab` / `↑` | próximo campo · anterior |
| | `Esc` | volta para a lista de áreas |

As teclas de ação só valem com a lista de áreas em foco, nas três áreas do
painel — dentro de um campo de texto, `u` é só a letra u. Com alteração
ainda não salva, o painel pede para salvar antes (`Ctrl+S`) em vez de
perdê-la.

## Status

A visão geral.

- **Acesso** — os três endereços que você abre no navegador, cada um com o
  estado atual e a latência; a **fila** do orquestrador (rodando, na espera,
  concluídas, falhas, e quantas tarefas aguardam o seu `/approve`); e o que
  está **em andamento** (ex.: `acme/loja#12`).
- **Containers** — cada serviço do projeto com o estado que o Docker reporta
  (`Up 2 hours (healthy)`, `Exited (1) 5 minutes ago`...), CPU e memória de
  agora e um mini-gráfico dos últimos minutos. Verde = rodando; amarelo =
  rodando mas com o healthcheck falhando; vermelho = parado.
- **Checklist** — o que precisa estar pronto para subir: arquivos de
  configuração, bootstrap feito, repositórios, portas livres, modelo de IA,
  acesso ao px0. `✗` bloqueia; `!` é só um aviso. A última linha diz o
  próximo passo ("tecle `u`", "Parado: px0 — tecle `u`"...).

Containers e fila atualizam a cada 5 segundos.

## Saúde

Cada serviço de que o OREIA-SECA depende — os dele e os de terceiros —
conferido de verdade (uma chamada real, não só "o container existe"):

| Serviço | Como é conferido | "Com problema" (▲) significa |
|---|---|---|
| **Docker** | `docker ps` | — |
| **Orquestrador** | `GET /health` na API | respondeu com erro HTTP |
| **Forgejo** | `GET /api/healthz` | respondeu com erro HTTP |
| **Postgres** | healthcheck do container | o container roda mas o banco não aceita conexão |
| **px0** | `GET /` (um redirecionamento para o login conta como no ar) | respondeu com erro HTTP |
| **LLM · código** | `GET <url>/models` com a sua chave | chave recusada, ou o **modelo configurado não está na lista do servidor** |
| **LLM · conversa** | idem — só aparece se for outro servidor ou outro modelo | idem |
| **Telegram** | `getMe` na API do Telegram; mostra o `@nome` do bot | token recusado |
| **MCP (HTTP)** | porta aberta | — |

Para cada um: latência da última checagem, um mini-gráfico do histórico
(`×` marca uma checagem sem resposta) e a porcentagem de checagens ok desde
que o painel abriu.

| Símbolo | Significa |
|---|---|
| `●` verde | respondeu como esperado |
| `▲` amarelo | respondeu, mas tem algo errado — o detalhe diz o quê |
| `✖` vermelho | não respondeu |
| `○` cinza | desligado ou não configurado (Telegram sem token, MCP HTTP desligado) — **não é um problema** |

**Precisa de atenção** repete, com o texto inteiro, o detalhe de tudo que
está `▲` ou `✖` — é onde ler o que fazer. Exemplos reais do que essa área
pega: um caractere a mais no nome do modelo (`...flash-1017``), o Ollama
fechado, uma chave de API vencida, o token do bot trocado no BotFather.

**Modelos, como o orquestrador os vê** mostra cada modelo de `[models.*]`:
o estado que o próprio orquestrador registrou (`ok`, `inacessível`, `não
configurado`) e quais papéis (`planner`, `dev-back`...) o usam. Se a tabela
de cima diz que o servidor do LLM está no ar mas aqui o modelo está
`inacessível`, o problema é de dentro do container para fora (rede, firewall,
`localhost` no lugar de `host.docker.internal`).

A saúde é conferida a cada 15 segundos — não a cada 5, para não ficar
batendo na API de terceiros — e na hora, com `r`. Nenhuma checagem gasta
tokens: nunca é feita uma chamada de geração.

## Uso

O consumo dos agentes, lido de `volumes/oreia-seca/data/escalation.jsonl` —
uma linha por tentativa de resolver uma fase de uma issue. Funciona com o
orquestrador parado, e mostra o histórico inteiro.

- **Totais** — execuções (e em quantas tarefas), taxa de sucesso, tokens
  (entrada e saída), custo e tempo médio por execução.
- **Período** — o mesmo recorte para as últimas 24 horas e os últimos 7
  dias, e um mini-gráfico de tokens por dia nos últimos 14 dias, com o pico.
- **Por modelo** e **por papel** — quantas execuções, taxa de sucesso, tokens,
  tempo médio e custo de cada um. É onde se vê se um modelo local dá conta ou
  se o papel `dev-back` precisa de um modelo melhor.
- **Últimas execuções** — as 12 mais recentes: quando, papel, modelo, degrau
  da escalada e executor, se passou, duração, tokens e o motivo quando falhou.

Sobre os números:

- **Custo** só aparece para degraus com preço configurado
  (`cost_per_mtok_in`/`cost_per_mtok_out` em `[escalation.*]`, veja
  [Executores e escalada](executores.md)). Modelo local custa zero e aparece
  como `—`.
- **Sucesso** = a tentativa passou na verificação do degrau (`test_cmd`,
  quando configurado). Uma fase que falha no primeiro degrau e passa no
  segundo conta como uma falha e um sucesso.
- A conversa direta (texto livre no Telegram) não entra aqui — só o trabalho
  dos agentes em issues.

No Telegram, `/stats [papel]` dá o agregado por papel a partir do mesmo
arquivo.

## Configurar

Sete áreas. `Enter` entra nos campos; `Ctrl+S` mostra o resumo e salva.
Campos que já tinham valor aparecem preenchidos; **segredos nunca são
mostrados** — só "(definido — digite para trocar)". Campo de texto deixado
vazio não muda o que já estava gravado. Número inválido (porta com letras)
aparece no resumo como erro, e nada é gravado.

| Área | O que edita |
|---|---|
| **px0** | quem pode entrar na revisão de PRs (`PX0_ALLOWED_USERS`) — vazio bloqueia todo mundo. Mostra se as credenciais do bootstrap existem |
| **Forgejo** | `[forgejo]`: URL interna e pública, repositórios, branch base, estilo de merge |
| **MCP** | liga/desliga o [servidor MCP](mcp.md) e o transporte HTTP (gera `MCP_TOKEN` sozinho); usuário, diretório de trabalho, rodadas de revisão, bind |
| **Telegram** | liga/desliga o canal; token do bot e IDs permitidos |
| **LLM** | URL do servidor, modelo de código, modelo de conversa, chave; contexto e temperatura de cada um. Ao sair do campo da URL, lista os modelos do servidor |
| **Executor** | executor padrão (`Espaço` alterna entre `builtin`, `opencode`, `codex`...) e os limites: rodadas por fase, retentativas, timeouts |
| **Servidor** | host e porta da API, diretórios de dados e workspace, concorrência e timeout da fila |

Ao salvar, o painel pergunta se aplica agora — isto é, se reinicia o
orquestrador para ele ler a configuração nova. Cada gravação deixa um backup
(`.env.bak`, `oreia-seca.toml.bak`), e os comentários do seu
`oreia-seca.toml` são preservados.

Depois de acrescentar um repositório em **Forgejo** (ou um agente, no
`oreia-seca.toml`), tecle `b` na área Status: o bootstrap cria só o que
falta. O que o painel ainda não edita — agentes, papéis, escalada, modelos
além de `coder`/`chat` — fica no `oreia-seca.toml`; veja
[Configuração](configuracao.md).

## Sem tela: `saude` e `status`

Para script, cron, CI, ou um terminal sem modo interativo:

```bash
./oreia saude       # a área Saúde, uma vez, em texto
./oreia status      # docker compose ps + checagem rápida
```

```
Saúde dos serviços do OREIA-SECA

  ✅ Docker              39 ms   4 de 4 container(s) do projeto rodando
  ✅ Orquestrador         4 ms   localhost:3120
  ✅ Forgejo             12 ms   localhost:3000
  ✅ Postgres             —      Up 2 hours (healthy)
  ✅ px0                  8 ms   localhost:7777
  ✅ LLM · código       603 ms   openrouter.ai · qwen/qwen3-coder · 431 modelo(s) no servidor
  ✅ Telegram          1116 ms   @meu_bot
  ⚪ MCP (HTTP)           —      transporte HTTP desligado — opcional (área MCP)
```

`./oreia saude` sai com código 1 se algum serviço configurado estiver fora
do ar ou com problema — dá para usar num cron: `./oreia saude || avisar`.

## Como as teclas de ação funcionam

O painel nunca chama `docker compose` por conta própria. Quando você tecla
`u`, ele grava "up" num arquivo (`.oreia-action`), fecha, e o launcher
`./oreia` — que roda no seu sistema, com o Docker de verdade — executa
`docker compose up -d` e reabre o painel.

Por que assim: sem `cargo` instalado (o caso normal), o painel roda dentro
de um container avulso. Ele recebe o `docker.sock` para **ler** o estado dos
containers (`docker ps`, `docker stats`), mas não tem o plugin `compose` nem
os caminhos do seu disco para resolver os volumes do `docker-compose.yml`.
Deixar toda ação com o launcher faz as teclas funcionarem igual em qualquer
sistema, com ou sem `cargo`.

Consequência: se você abrir a tela chamando o binário direto
(`cargo run -p oreia-seca -- setup`), não há launcher do outro lado — as
teclas de ação só dizem qual comando rodar no terminal. Use `./oreia`.

Sobre o `docker.sock` nesse container: ele é montado só no container
**avulso e temporário** do painel, que você mesmo abre e que não executa
código gerado por modelo. O serviço do orquestrador — onde os agentes rodam
comandos — **não** recebe o socket (a menos que você ative os
[containers por executor](executores.md#containers-por-executor)). Veja
[Segurança](seguranca.md).
