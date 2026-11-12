# Segurança

Agentes de IA executam comandos gerados por um modelo. Trate o container do
OREIA-SECA como uma máquina onde código não confiável roda. O que já está em vigor:

| Risco | Proteção |
|---|---|
| Alguém usar seu bot do Telegram | só IDs em `TELEGRAM_ALLOWED`; lista vazia bloqueia todos |
| Webhook falso disparando agentes | assinatura HMAC-SHA256 obrigatória; sem segredo configurado, o endpoint recusa tudo |
| Chamadas não autorizadas à API | Bearer token em `/api/*` (comparação em tempo constante) |
| Tarefas automáticas fazendo algo sem você ver | fontes `rest` e `cron` esperam `/approve` (configurável) |
| Modelo lendo segredos | o shell das ações roda com **ambiente limpo**: tokens e chaves do `.env` não chegam lá |
| Modelo usando o token git | o token vai num cabeçalho por comando (`git -c http.extraHeader`), nunca gravado em `.git/config`; push só pelo orquestrador |
| Escrita fora da área da tarefa | `file_read`/`file_write` recusam caminhos absolutos e `..` |
| Injeção de comando por skills | parâmetros chegam como variáveis de ambiente (`$P_NOME`), não interpolados no script |
| Código ruim no `main` | branch protegido: sem push direto, merge exige sua aprovação, revisão rejeitada bloqueia |
| Agentes acionando uns aos outros em loop | eventos de contas de agente são ignorados; um agente por issue |
| Reviewer "aprovando" ou fazendo merge sozinho | ele só comenta (`comment_issue`); `review_pull`/`merge_pull` (aprovar, reprovar, fazer merge) só existem atrás de `/merge`/`/changes`, comandos seus, com o seu token — nenhum agente tem esse caminho de código |
| Comando travado | timeout por comando de shell, por chamada de modelo e por tarefa |
| `OREIA_SECA_LLM_DEBUG=1` logar prompt/código do usuário | desligado por padrão; a chave da API nunca entra no log (vai só no cabeçalho HTTP), mas o conteúdo das mensagens sim — ative só para depurar, nunca em produção com dados sensíveis |
| CLI externa (aider/opencode/goose/codex) agindo sem confirmação | é intencional: os quatro presets passam a flag que pula a aprovação própria da ferramenta (`--yes-always`, `--auto`, `--dangerously-bypass-approvals-and-sandbox`...) — a contenção real é a nossa (`sandboxed_shell`: ambiente limpo, workdir confinado, timeout, desfaz tentativa que falha), não a da ferramenta. Um executor externo roda com as mesmas garantias do `builtin`, nenhuma a menos |
| Containers por executor (`container` em `[executors.*]`) exigem `/var/run/docker.sock` montado | isso é, na prática, acesso root ao HOST — quem controla o socket do Docker consegue montar qualquer caminho e escapar do próprio container. **Atenção:** como o executor `builtin` roda no mesmo container do `oreia-seca` e o `sandboxed_shell` mantém o PATH, qualquer comando gerado pelo modelo no `builtin` pode falar com o socket (`docker run` ou `curl --unix-socket`) e virar root no host enquanto o override estiver montando o socket neste container. Veja [Executores e escalada](executores.md#containers-por-executor) |
| O painel (`./oreia`) recebe `/var/run/docker.sock` | só o container **avulso e temporário** do painel/assistente, que você abre e fecha — não o serviço do orquestrador, onde os agentes rodam comandos. Ele não executa nada gerado por modelo e usa o socket só para ler (`docker ps`/`docker stats`); subir, parar e bootstrap são feitos pelo launcher, no host. Com `cargo` instalado nem container existe: o binário roda nativo. Quem não quer o socket nem ali pode usar `./oreia status`/`./oreia saude` (texto) e editar os arquivos na mão. Veja [Painel](painel.md#como-as-teclas-de-ação-funcionam) |
| [px0](px0.md) guarda o mesmo token de escopo total (`FORGEJO_OWNER_TOKEN`) que `/merge`/`/changes` já usam | não é um segredo novo nem um escopo maior — é o mesmo nível de confiança. O px0 em si não tem login nenhum e por isso não é exposto diretamente: só o `launcher.py` (porta 7777) é publicado, exige entrar com a conta do Forgejo (OAuth2 — login único, mesma sessão do Forgejo — ou usuário/senha manual) e só então faz proxy pro px0 — sem sessão, `/review/` redireciona pro login. Só contas em `PX0_ALLOWED_USERS` conseguem criar sessão (vazio = ninguém, igual `TELEGRAM_ALLOWED`); qualquer uma delas herda o mesmo acesso do dono (o login controla quem usa a ferramenta, não qual token ela usa por dentro) |

## Servidor MCP

O [servidor MCP](mcp.md) (P.O. por IA + desenvolvedor sob demanda) é uma segunda via
de trabalho, explícita e manual — nunca disputa a mesma issue com a via automática
(posse persistente, `data/claims.json`). Riscos específicos dela:

| Risco | Proteção |
|---|---|
| Ferramenta MCP devolvendo um token sem querer | nenhuma resposta de ferramenta carrega segredo — testado com valor sentinela; o que volta é só o corpo que o Forgejo/`oreia-seca` já filtram |
| `submit_issue` empurrando pra `main` | recusa explicitamente se o branch atual for a base, antes de qualquer `git push` |
| Commit/push do MCP usando token errado | conta própria (`agent-mcp`, `[mcp].agent_token_env`) — nunca a de um `[[agents]]`; o token vai no mesmo cabeçalho HTTP por comando que os agentes já usam, nunca gravado em `.git/config` |
| Processo MCP lendo/escrevendo o estado direto | ele só fala com o `oreia-seca` já rodando pela API interna (`/api/mcp/*`, mesmo Bearer de `/api/*`) — nunca acessa `data/`/`workspace` por fora |
| Ação sem rastro | toda ferramenta que escreve registra em `data/mcp.jsonl` (ferramenta, repo, issue, resultado, timestamp — nunca o conteúdo da issue/comentário/diff) |
| `review_pr` virando um loop agente↔MCP | `[mcp].max_review_rounds` (padrão 3) por PR; ao estourar, recusa e avisa os canais humanos — decisão passa a ser sua |
| Transporte HTTP opcional (`[mcp].http`) exposto sem querer | desligado por padrão; liga só com `MCP_TOKEN` definido (sem ele, nem sobe); bind padrão `127.0.0.1`; `Origin` fora de uma allowlist de hosts locais é recusado |
| `dispatch_issue`/`review_pr` forçando um degrau caro ou pulando aprovação | `ask_before_step`/`max_cost_usd` do papel continuam valendo normalmente a partir do degrau forçado — a ferramenta não contorna nenhum dos dois |

## O que você ainda precisa decidir

- **Isolamento por agente.** O executor `builtin` ainda roda dentro do mesmo container do OREIA-SECA, em diretórios separados por tarefa. Com o override de `docker.sock` ativo para executores externos, o shell do `builtin` tem acesso ao socket do host se estiver no mesmo container — para isolar de verdade, o acesso ao socket deve ficar em serviço separado (launcher restrito que só lança imagens `oreia-seca-exec-*`) ou o shell do `builtin` deve ser conteinerizado.
- **Rede de saída.** O shell tem acesso à internet (para `npm install`, etc.). Se quiser restringir, coloque o container numa rede com proxy de saída.
- **Exposição.** Não exponha as portas 3000, 3120 e 7777 na internet. Para acesso remoto, use VPN (Tailscale/WireGuard). O Telegram funciona sem expor nada, porque usa long polling.
- **Escopo dos tokens.** Os agentes recebem `write:repository` e `write:issue`. O seu token (`FORGEJO_OWNER_TOKEN`) tem escopo total: guarde o `.env` com cuidado.
- **`reviewer` pode fazer push na branch de outro agente.** Pra corrigir um problema pequeno, ele comita e sobe na mesma branch do PR (ex.: `agent/dev-back/issue-7`), com o próprio token — não é a branch dele. Se isso for demais, remova `reviewer` do `[[agents]]` (a revisão automática simplesmente não roda) ou restrinja o escopo do `AGENT_REVIEWER_TOKEN` no Forgejo.
