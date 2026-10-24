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
| comentar `@agent-back <pedido>` na issue | o agente volta ao mesmo branch e atualiza o PR |
| comentar `@agent-back <pedido>` no PR do agente | idem |
| `/changes <n> <pedido>` no Telegram | registra "mudanças solicitadas" no PR e aciona o agente |

Proteções:
- eventos gerados por contas de agente são ignorados (um agente não aciona outro em loop);
- um mesmo `repo#issue` só tem um agente trabalhando por vez.

## Coordenação front ↔ back

Agentes em repositórios ou issues diferentes não conversam entre si. Para que front e back concordem:

- **Contrato primeiro.** A primeira issue de uma feature define o contrato (OpenAPI ou tipos compartilhados) e as demais o referenciam. As instruções de `dev-back` e `dev-front` já pedem para seguir o contrato.
- **Monorepo ajuda.** Com front e back no mesmo repositório, os dois agentes enxergam o contrato e o CI testa a integração.

## O revisor

`reviewer` é um agente como os outros: rode-o colocando a label `agent:review` numa issue que descreva o que revisar. Ele roda testes e lint, corrige o que for pequeno e escreve `REVIEW.md`. Revisão automática de todo PR é o próximo passo do [roadmap](roadmap.md).
