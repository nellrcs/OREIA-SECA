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
| Comando travado | timeout por comando de shell, por chamada de modelo e por tarefa |

## O que você ainda precisa decidir

- **Isolamento por agente.** Hoje todos os agentes executam dentro do mesmo container do OREIA-SECA, em diretórios separados por tarefa. O shell do modelo pode, em tese, ler outros diretórios do container. Para isolamento forte, rode cada execução num container efêmero (ou LXC no Proxmox) — está no [roadmap](roadmap.md).
- **Rede de saída.** O shell tem acesso à internet (para `npm install`, etc.). Se quiser restringir, coloque o container numa rede com proxy de saída.
- **Exposição.** Não exponha as portas 3000 e 3120 na internet. Para acesso remoto, use VPN (Tailscale/WireGuard). O Telegram funciona sem expor nada, porque usa long polling.
- **`oreia-seca setup --web` não tem autenticação.** Pensado para `127.0.0.1` (o padrão): só a própria máquina acessa, e o servidor desliga sozinho depois do primeiro envio do formulário. Se ligar noutro host (`--host 0.0.0.0`), qualquer um que alcançar a porta 3121 enquanto ela estiver aberta grava o `.env` dessa máquina — use só numa rede que você confia, nunca exposto na internet.
- **Escopo dos tokens.** Os agentes recebem `write:repository` e `write:issue`. O seu token (`FORGEJO_OWNER_TOKEN`) tem escopo total: guarde o `.env` com cuidado.
