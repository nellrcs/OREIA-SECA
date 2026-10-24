# Roadmap

## Pronto (v0.1)

- [x] Orquestrador em Rust com fila, aprovação cross-canal, classificador e sessões
- [x] Roteador de modelos por papel com reserva, timeout e cooldown
- [x] Qualquer modelo OpenAI-compatível (Ollama, LM Studio, llama.cpp, vLLM, OpenRouter, Gemini)
- [x] Telegram com botões de aprovação e whitelist; terminal; API REST
- [x] Webhook do Forgejo com HMAC; agentes por label; `@menção` para ajustes
- [x] Clone → fases → commit/push → PR → comentário na issue, com a conta do agente
- [x] `/prs`, `/merge`, `/changes` pelo Telegram
- [x] Skills em Markdown + shell; ações confinadas ao diretório da tarefa
- [x] Bootstrap do Forgejo (usuários, tokens, repos, labels, webhook, proteção do `main`)
- [x] 72 testes, incluindo o fluxo issue → PR com modelo falso, Forgejo simulado e git real

## Pronto (v0.2)

- [x] Executor de fases plugável (`builtin` + CLIs externas: Aider, OpenCode, Goose, Codex)
- [x] Escada de escalada por papel: degraus com verificação por `test_cmd`, aprovação a partir de um degrau e teto de custo — veja [Executores e escalada](executores.md)
- [x] `data/escalation.jsonl` e `/stats [papel]`

## Pronto (v0.3)

- [x] LLM como serviço externo configurável: `docker-compose.yml` nunca sobe modelo junto — sempre um servidor externo (outra máquina, LM Studio, OpenRouter...)
- [x] `base_url_env`/`model_env` em `[models.*]`, com boot tolerante a modelo não configurado ou inacessível
- [x] `oreia-seca setup llm` (`make llm-setup`): assistente interativo ou por flags que lista modelos do servidor e grava o `.env`
- [x] `/models` e `GET /api/status` mostram o estado de cada modelo (`não configurado` | `inacessível` | `ok`), sem vazar chaves

## Pronto (v0.4)

- [x] `oreia-seca setup` unificado: Telegram, LLM e bootstrap do Forgejo numa coisa só, de três formas — veja [Instalação](instalacao.md)
- [x] Wizard de terminal (`make setup`), página web local sem autenticação (`make setup-web`) e arquivo de respostas/flags não interativo (`make setup-answers`), todos chamando a mesma `apply_answers`
- [x] `setup.answers.example.toml`: o formato pensado para automatizar ou para um agente de IA preencher e aplicar sozinho

## Próximos passos

1. **Validar ponta a ponta com modelo real.** Rodar `make setup` contra um servidor de verdade, criar uma issue pequena e ajustar prompts, `max_steps` e a escalada com base no que os modelos acertarem e errarem.
2. **Revisão automática.** Ao abrir um PR de `dev-back`/`dev-front`, acionar o `reviewer` no mesmo branch antes de te notificar.
3. **Planner que cria issues.** `/feature` com Forgejo configurado → o planner quebra em issues com as labels certas (back/front), contrato primeiro.
4. **Isolamento por execução.** Rodar cada tarefa num container efêmero (ou LXC no Proxmox) em vez de diretórios no mesmo container.
5. **Previews por PR.** Validar o template `templates/app-repo` com a stack do seu sistema e devolver o link do preview no Telegram (`/preview <n>`).
6. **Limites de iteração entre agentes.** Contar quantas vezes um PR voltou para ajustes e pedir sua intervenção a partir de N.
7. **Portar o que falta do OREIA-SECA** (veja [vindo-do-oreia-seca.md](vindo-do-oreia-seca.md)).
