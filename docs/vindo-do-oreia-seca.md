# Vindo do OREIA-SECA

Esta reescrita em Rust mantém os conceitos do OREIA-SECA original (Node.js) e muda a implementação.

## Equivalências

| OREIA-SECA v1 (Node) | OREIA-SECA v2 (Rust) |
|---|---|
| `maker/index.js` (`Maker`) | `crates/oreia-seca/src/maker/` |
| `maker/model-router.js` (`ModelRouter`, `ResilientModel`) | `oreia-llm::ModelRouter` |
| `models/lmstudio.js`, `openrouter.js`, `gemini.js`, `local.js` | um só `OpenAiCompat` |
| `maker/task-queue.js` | `oreia-core::queue::TaskQueue` |
| `maker/approval.js` (`ApprovalBroker`) | `oreia-core::approval::ApprovalBroker` |
| `maker/planner.js` | `crates/oreia-seca/src/planner.rs` |
| `maker/executor.js` | `crates/oreia-seca/src/maker/exec.rs` |
| `actions/parser.js` | `oreia-core::actions::parser` |
| `actions/shell.js`, `filesystem.js` | `oreia-core::actions::builtin` |
| skills `.md` com bloco JavaScript | skills `.md` com bloco `sh` |
| `inputs/telegram.js`, `terminal.js` | `oreia-channels` |
| `inputs/rest.js` | `crates/oreia-seca/src/http.rs` |
| `storage/task-store.js`, `session-store.js` | `oreia-core::store`, `oreia-core::session` |
| `/approve` `/reject` `/pending` `/status` `/queue` `/tasks` `/retry` `/skills` `/resumir` | mesmos comandos |

## O que mudou e por quê

- **Provedores.** Em vez de uma classe por provedor, um cliente OpenAI-compatível. LM Studio, Ollama, OpenRouter e Gemini falam esse formato, então menos código e trocar de provedor é só mudar `base_url`.
- **Reserva com cooldown.** No original, um modelo marcado offline ficava offline. Aqui volta sozinho depois de `cooldown_secs`.
- **Loop por fase.** O executor original fazia uma chamada por fase. Agora o modelo vê o resultado das ações e pode corrigir dentro da fase, até `max_steps`.
- **Skills em shell.** Executar JavaScript gerado dentro do processo (como `new AsyncFunction`) não tem equivalente seguro em Rust. As skills passam a ser scripts `sh` com parâmetros em variáveis de ambiente, o que também elimina injeção por interpolação.
- **Parser mais estrito em um ponto.** O formato `chave: valor` só é usado quando não há tags de parâmetro, para não "inventar" parâmetros a partir do conteúdo de arquivos YAML.
- **Classificador.** Palavras de escopo são comparadas por palavra inteira (`com` não casa mais com `comando`) e há mais verbos e substantivos.
- **Telegram sem framework.** Bot API direta com long polling. Lista de IDs vazia agora bloqueia todos (antes liberava).
- **Config em TOML** com validação: papéis apontando para modelos inexistentes ou campos digitados errado falham no boot.

## Ainda não portado

- `/carregar` e `/resumos` (os resumos já são salvos em `data/resumos/`).
- Barra de tokens colorida do modo debug (os tokens aparecem com `RUST_LOG=oreia_llm=debug`).
- `reload()` de modelos do LM Studio após cada fase.
- Skills específicas (browser, docker, cron, telegram_send).
