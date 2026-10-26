# Agentes

Um agente é a soma de quatro coisas:

1. **Uma conta no Forgejo** (`user`), com token de escopo mínimo (`write:repository`, `write:issue`, `read:user`).
2. **Uma label** (`label`) que o aciona quando aparece numa issue.
3. **Um papel de modelo** (`model_role`, padrão = `name`) no `[roles]`.
4. **Instruções** (`prompt` → `agents/<nome>/AGENT.md`), injetadas no planner e no executor.

## Criar um agente novo

```toml
# oreia-seca.toml
[roles]
dev-mobile = ["qwen-coder"]

[[agents]]
name = "dev-mobile"
label = "agent:mobile"
user = "agent-mobile"
token_env = "AGENT_DEV_MOBILE_TOKEN"
prompt = "agents/dev-mobile/AGENT.md"
```

```bash
mkdir agents/dev-mobile && $EDITOR agents/dev-mobile/AGENT.md
make bootstrap      # cria a conta, o token, a label e o acesso ao repositório
docker compose restart oreia-seca
```

Escreva o `AGENT.md` em inglês: modelos locais menores seguem instruções em inglês com mais precisão. As respostas para você continuam em português.

## Como acionar

| Ação | Resultado |
|---|---|
| issue nova com a label do agente | o agente planeja, trabalha e abre PR |
| adicionar a label a uma issue existente | idem |
| `/issue [dono/repo] <agente> [--base <branch>] <título>` no Telegram | abre a issue (com a label do agente), **sem descrição** — numa linha só não dá pra separar as duas, e é a descrição que o agente lê de verdade. Com `--base <branch>`, o PR sai contra essa branch em vez da padrão (mesma coisa que o campo "Ramo" abaixo) |
| `/issue [dono/repo] <título>` no Telegram (sem agente) | idem, sem label e sem descrição |
| `/issue` no Telegram, sem nada | pergunta repositório (se houver mais de um), depois o agente por botão, depois o título e **a descrição** por mensagem, depois a **branch base** — listada como botão (uma por branch do repo, mais "📌 Usar padrão"), com o pedido por texto livre como reserva se a listagem falhar |
| `/assign [dono/repo] <n> <agente>` no Telegram | adiciona a label do agente numa issue já aberta (sua ou criada sem agente) — aciona na hora, igual arrastar a label na UI |
| `/assign` no Telegram, sem nada | pergunta repositório, depois o número da issue (por mensagem), depois o agente por botão |
| `/prs` no Telegram | um cartão por PR aberto, com botões "✅ Merge" e "📝 Pedir ajuste" (este último pede o texto por mensagem) |
| comentar `@agent-back <pedido>` na issue | o agente volta ao mesmo branch e atualiza o PR |
| comentar `@agent-back <pedido>` no PR do agente | idem |
| `/changes <n> <pedido>` no Telegram | registra "mudanças solicitadas" no PR e aciona o agente |
| agente abre um PR (qualquer um) | se `reviewer` estiver configurado, ele entra sozinho e comenta — veja [O revisor](#o-revisor) |

Proteções:
- eventos gerados por contas de agente são ignorados (um agente não aciona outro em loop);
- um mesmo `repo#issue` só tem um agente trabalhando por vez.

`/issue` e `/assign` usam o token do dono (`[forgejo].owner_token_env`) e, quando um agente é
informado, exigem a label dele já criada no repositório (Forgejo não deixa criar/adicionar label
por nome, só por ID — a API só aceita o ID de uma label que já existe). Se a label não existir
ainda, crie-a uma vez em Issues → Labels. Em `/issue`, o agente é identificado pelo primeiro
token depois do repo batendo com um `name` de `[[agents]]`; se não bater com nenhum, tudo depois
do repo entra como título e a issue sai sem label.

**Branch base.** Por padrão o agente clona, cria a própria branch e abre o PR contra
`[forgejo].base_branch` (`main`, geralmente). Pra trabalhar em cima de uma feature branch já
existente em vez de `main`, preencha o campo **"Ramo"** ao abrir a issue no Forgejo (fica perto
de Label/Marco, na barra lateral) com o nome dessa branch — o agente passa a clonar, commitar e
abrir o PR contra ela. Pelo Telegram é o mesmo campo: `--base <branch>` no `/issue` direto, ou o
passo de branch base do assistente (`/issue` sozinho) — que lista as branches do repo como
botão em vez de pedir pra digitar de cor. A branch precisa já existir no repositório — nem o
Forgejo nem o `/issue` criam uma nova. O `reviewer`, ao comentar
automaticamente num PR, sempre compara contra a base REAL daquele PR (seja `main` ou a branch
escolhida), não precisa de nenhuma configuração extra.

## Coordenação front ↔ back

Agentes em repositórios ou issues diferentes não conversam entre si. Para que front e back concordem:

- **Contrato primeiro.** A primeira issue de uma feature define o contrato (OpenAPI ou tipos compartilhados) e as demais o referenciam. As instruções de `dev-back` e `dev-front` já pedem para seguir o contrato.
- **Monorepo ajuda.** Com front e back no mesmo repositório, os dois agentes enxergam o contrato e o CI testa a integração.

## O revisor

Um agente chamado exatamente `reviewer` (esse nome é especial) entra sozinho sempre que outro agente
abre um PR: analisa a branch que já existe (não cria uma nova), roda build/lint/teste, corrige só o
que for pequeno e óbvio, e **comenta no PR** com o que achou. Sem `reviewer` configurado, nada disso
roda — o fluxo continua igual, só sem a revisão automática.

O comentário do reviewer nunca é um veredito: ele não aprova, não reprova e não faz merge — não tem
acesso para isso (`review_pull`/`merge_pull` só são chamados por `/merge`/`/changes`, com o seu
próprio token, nunca com o de um agente). A decisão final é sempre sua. Veja
[Segurança](seguranca.md).

Você também pode @mencionar `@agent-reviewer <pedido>` numa issue ou PR, como qualquer outro agente,
para pedir uma segunda olhada fora do gatilho automático.
