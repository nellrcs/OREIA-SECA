# Solução de problemas

Comece sempre por aqui:

```bash
./oreia doctor      # confere Docker, portas, arquivos e a saúde de cada serviço
./oreia logs        # logs do orquestrador (./oreia logs forgejo, px0, postgres...)
```

No painel (`./oreia`), a área **Saúde** mostra o que está fora do ar e, em
"Precisa de atenção", o que fazer. Veja [Painel](painel.md#saúde).

- [Instalação](#instalação)
- [Subir os serviços](#subir-os-serviços)
- [Modelo de IA (LLM)](#modelo-de-ia-llm)
- [Forgejo e px0](#forgejo-e-px0)
- [Telegram](#telegram)
- [Painel e terminal](#painel-e-terminal)
- [Os agentes não fazem nada](#os-agentes-não-fazem-nada)

## Instalação

**"Docker não encontrado" / "O Docker está instalado mas não está rodando"**
Instale o Docker ([Requisitos](instalacao.md#requisitos)) ou, se já instalou,
abra o Docker Desktop e espere o ícone parar de animar. No Linux:
`sudo systemctl start docker`.

**Linux: "permission denied" ao falar com o Docker**
Seu usuário não está no grupo `docker`:
`sudo usermod -aG docker $USER`, depois saia da sessão e entre de novo (ou
reinicie).

**Windows: "Não encontrei o Git for Windows"**
O `oreia.cmd` usa o bash que vem com o
[Git for Windows](https://git-scm.com/download/win). Instale com as opções
padrão e rode de novo.

**Windows: `.\oreia` abre uma janela "Como deseja abrir este arquivo?"**
Use `oreia.cmd` (com a extensão). `oreia` sem extensão é o script bash — só
roda de dentro do Git Bash, como `./oreia`.

**Windows (Git Bash): "the input device is not a TTY"**
Aconteceu ao abrir o painel numa janela onde o launcher não detectou o
`winpty`. Force com `OREIA_WINPTY=1 ./oreia`. O contrário — tela embaralhada
no Windows Terminal — resolve com `OREIA_WINPTY=0 ./oreia`.

**A primeira instalação demora muito na etapa 1**
É a compilação do programa (5 a 15 minutos, uma vez só). As próximas
aberturas usam a imagem pronta. Se foi interrompida, rode `./oreia install`
de novo: o Docker aproveita o que já baixou.

**"a instalação parou no meio"**
Leia a mensagem logo acima dela, corrija (em geral: porta ocupada, internet,
Docker parado) e rode `./oreia install` outra vez — ele continua de onde
parou. Os casos comuns estão nas seções abaixo.

**"sem terminal interativo para o assistente"**
Você está rodando por um script, CI ou agente. Use
`./oreia install --answers setup.answers.toml` — veja
[Instalação automatizada](instalacao.md#instalação-automatizada).

## Subir os serviços

**"a porta 3000 (…) já está em uso por outro programa"**
Outro programa (ou outro container) usa essa porta. O assistente de
instalação já desvia sozinho para a próxima livre; numa instalação existente,
mude no `.env` a variável indicada (`FORGEJO_HTTP_PORT`, `FORGEJO_SSH_PORT`,
`OREIA_SECA_PORT`, `PX0_PORT`) e rode `./oreia up`. Se mudar a porta do
Forgejo, ajuste também a "URL pública" na área **Forgejo** do painel — é o
link que aparece nas mensagens. A comunicação interna entre os containers
não é afetada.

**`required variable FORGEJO_REPOS is missing a value`** (ou
`FORGEJO_DB_PASSWORD`)
O `.env` existe mas o bootstrap ainda não rodou. Rode `./oreia install`.

**`required variable DOCKER_GID is missing a value`**
Existe um `docker-compose.override.yml` (containers por executor) e o `.env`
foi recriado sem `DOCKER_GID`/`OREIA_SECA_HOST_DIR`. Defina os dois de novo
([Executores](executores.md#containers-por-executor)) ou apague o override.

**"o Forgejo ainda não foi preparado (falta tls/ca.crt)"**
`./oreia up` foi chamado antes do bootstrap. Rode `./oreia install` (ou
`./oreia bootstrap`).

**Um container fica reiniciando**
`./oreia logs <serviço>` mostra o motivo. O mais comum no `oreia-seca` é o
`oreia-seca.toml` inválido depois de uma edição manual: `./oreia check`
aponta a linha.

## Modelo de IA (LLM)

A área **Saúde** separa os três casos:

**`✖ LLM: sem resposta`** — o servidor do modelo não está acessível.

- Ollama/LM Studio **neste computador**: o endereço precisa ser
  `http://host.docker.internal:11434/v1` (ou `:1234/v1` no LM Studio), nunca
  `localhost` — dentro do container, `localhost` é o próprio container.
- **Linux**, Ollama neste computador: por padrão o Ollama só escuta em
  `127.0.0.1`, e o container chega pela rede do Docker. Faça-o escutar em
  todas as interfaces: `OLLAMA_HOST=0.0.0.0` (em
  `systemctl edit ollama` → `Environment="OLLAMA_HOST=0.0.0.0"`, depois
  `systemctl restart ollama`).
- LM Studio: na aba *Developer*, o servidor precisa estar **ligado** e, para
  outra máquina alcançar, com "Serve on Local Network" marcado.
- Outra máquina: confira o IP e o firewall dela (portas 11434/1234).

**`▲ LLM: modelo "…" não está na lista do servidor`** — o servidor responde,
mas o nome do modelo está errado (erro de digitação, um caractere a mais,
modelo não baixado). Corrija na área **LLM**: ao sair do campo da URL, o
painel lista os nomes exatos. No Ollama, `ollama list` mostra o que está
baixado.

**`▲ LLM: chave da API recusada`** — chave errada, vencida ou sem crédito.
Troque na área **LLM** (campo "Chave da API").

**Na tabela "Modelos, como o orquestrador os vê" aparece `inacessível`, mas a
linha do LLM em cima está verde** — o servidor responde, mas o orquestrador
marcou o modelo como fora depois de uma falha e ele fica de molho por
`[llm].cooldown_secs` (120 s por padrão). Volta sozinho; `./oreia restart`
adianta.

Mais em [Modelos locais](modelos-locais.md).

## Forgejo e px0

**Qual é a minha senha do Forgejo?**
`FORGEJO_OWNER_PASSWORD`, no `.env`. O usuário é `FORGEJO_OWNER_USER`
(padrão `dono`).

**px0: "conta não autorizada" / não consigo entrar em `localhost:7777`**
`PX0_ALLOWED_USERS` vazio bloqueia todo mundo. Preencha na área **px0** do
painel com o seu usuário do Forgejo e aplique. (O assistente de instalação já
faz isso; instalações antigas podem não ter.)

**px0: "Client ID not registered"**
O app OAuth2 do px0 não foi registrado no Forgejo, ou a porta do px0 mudou
depois do registro. Apague `PX0_OAUTH_CLIENT_ID` e `PX0_OAUTH_CLIENT_SECRET`
do `.env` e rode `./oreia bootstrap` — ele registra de novo com a porta
atual. Veja [px0](px0.md).

**Acrescentei um repositório/agente e ele não existe no Forgejo**
Tecle `b` no painel (ou `./oreia bootstrap`): cria só o que falta — conta,
token, label, acesso, webhook.

**Quero trocar o nome do dono**
Só recomeçando: `./oreia reset` e `./oreia install`
([Recomeçar do zero](instalacao.md#recomeçar-do-zero)).

## Telegram

**`▲ Telegram: token recusado`** — o token não confere com o do
[@BotFather](https://t.me/BotFather) (`/mybots` → seu bot → *API Token*).
Troque na área **Telegram**.

**O bot não responde** — só IDs em `TELEGRAM_ALLOWED` são atendidos. Mande
qualquer mensagem: ele responde com o seu ID; coloque-o na área **Telegram**
e aplique. Confira também se o canal está ligado (`●` ao lado de Telegram).

## Painel e terminal

**Os mini-gráficos aparecem como quadradinhos ou "?"**
A fonte do terminal não tem os caracteres de bloco. No Windows, use o
*Windows Terminal* (ou uma fonte como Cascadia Mono/Consolas); no cmd antigo,
rode `chcp 65001` antes.

**As teclas `u`/`d`/`b` só mostram um aviso ("aberta sem o launcher")**
A tela foi aberta chamando o binário direto. Abra por `./oreia` — é o
launcher que executa as ações. Veja
[Como as teclas de ação funcionam](painel.md#como-as-teclas-de-ação-funcionam).

**Área Status: "Docker não encontrado" ou "não está respondendo"**
O painel não conseguiu falar com o Docker. Abra o Docker Desktop (ou inicie o
serviço) e tecle `r`. No Mac, se o erro persistir só no painel, ative
*Settings → Advanced → Allow the default Docker socket to be used* no Docker
Desktop.

**Área Uso vazia**
Normal antes da primeira execução de um agente — o registro
(`volumes/oreia-seca/data/escalation.jsonl`) nasce na primeira tentativa.

**Linux: `.env` ficou com dono `root`**
Aconteceu em versões anteriores, quando o painel rodava como root no
container. Corrija uma vez: `sudo chown $USER: .env oreia-seca.toml`. As
versões atuais devolvem a posse sozinhas.

## Os agentes não fazem nada

Criou a issue com a label e nada aconteceu? Confira, nesta ordem:

1. **Área Saúde**: orquestrador, Forgejo e LLM estão `●`?
2. **A label** é exatamente a do agente (`agent:back`, `agent:front`,
   `agent:review` — veja `[[agents]]` no `oreia-seca.toml`)?
3. **O repositório** está em `[forgejo].repos` (área **Forgejo**) e o
   bootstrap rodou depois de ele ser acrescentado?
4. **Fila** (área Status): a tarefa está "na espera"? Com
   `[queue].concurrency = 1` roda uma por vez. "Aguardando /approve"? Tarefas
   vindas de REST/cron esperam a sua aprovação.
5. **Logs**: `./oreia logs` — erros de webhook (assinatura), de token do
   agente ou do modelo aparecem aqui.

Se a tarefa roda mas o resultado é ruim (o agente não termina, erra o
código), o caminho é outro: modelo melhor para o papel, ou uma escada de
escalada — veja [Executores e escalada](executores.md) e a área **Uso**, que
mostra a taxa de sucesso por modelo.
