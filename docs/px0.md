# px0 (revisão de PR)

[px0](https://px0.ai/docs/) é um workbench de revisão/IDE focado em código gerado por IA. É o
lugar onde você faz a aprovação manual de PR (`Approve`/`Request Changes`/`Merge`), em vez do
`/merge`/`/changes` do Telegram ou da UI do Forgejo.

**Visual.** Inspirado no [ForgeUI](https://forgeui.in/) — mas sem adotar os componentes dele de
verdade: ForgeUI é código **React** (Next.js + Tailwind + shadcn/ui + Motion/GSAP), feito pra
copiar num projeto com build step (npm). `launcher.py` é um único arquivo Python, biblioteca
padrão, sem Node — trocar a arquitetura toda por um front-end React só pra ter esse visual seria
desproporcional. Em vez disso: as **cores são as mesmas de verdade** — extraí os tokens OKLCH do
tema escuro ("neutral", padrão shadcn/ui) direto do CSS compilado de `forgeui.in` (não um
palpite de paleta), mais fundo com grade sutil + dois blobs de glow radial flutuando (o mesmo
tipo de `bg-gradient-to-*`/`blur-3xl` que o bundle deles usa), **Tailwind CSS via CDN** (sem
build) pro layout, **Chart.js via CDN** pro gráfico de modelos, fonte Inter (Google Fonts), e
animações CSS/Tailwind puras (fade-in escalonado nos cards, pulso no indicador "ao vivo", números
que saltam quando mudam, spinner no restart).

**Status em tempo real de verdade.** A página não recarrega mais a cada 5s (era um
`<meta http-equiv="refresh">` — piscava e reiniciava as animações). Agora `/status.json` devolve
um payload só (fila/papéis/modelos + atividade + containers), e um `fetch()` em JS atualiza os
números/gráfico/listas no lugar, sem piscar.

**Dashboard mais completo.** Tirei o gráfico de rosca de "modelos por estado" (não agregava
nada que a lista de modelos já não mostrasse) e no lugar entraram três coisas novas, todas lendo
dados que já existiam mas não apareciam na tela:

- **Atividade recente**: as últimas tentativas de fase, lidas de `data/escalation.jsonl` (o
  mesmo arquivo que o `/stats` do Telegram já usa) — papel, fase, degrau, **qual executor** e
  **modelo** foram usados, se passou ou falhou (e por quê, se falhou), e **quanto tempo levou**,
  com "há Xs/min/h" relativo. Atualiza a cada 5s junto com o resto.
- **Duração das últimas tentativas**: gráfico de barras (Chart.js) com o tempo de cada uma das
  últimas tentativas, verde se passou e vermelho se falhou — a versão visual do mesmo log.
- **Containers**: CPU% e memória (barra + uso real, ex. "63.1MiB / 15.59GiB") de cada container
  do projeto, via `docker stats`. Opt-in — mesmo `docker.sock` já usado pela Configuração; sem
  ele montado, mostra uma mensagem em vez de quebrar. Cada card recebe o selo **executor** quando
  é um container efêmero de opencode/codex (`docker run --rm`, sem nome fixo — identificado pela
  imagem `oreia-seca-exec-*`, já que o Docker dá um nome aleatório a eles). Como são descartáveis
  e vivem só durante uma tentativa de fase, só aparecem se a amostragem acontecer enquanto uma
  tarefa está rodando — é uma foto do instante, não uma garantia de que todo executor vai aparecer.
- **Memória ao longo do tempo**: gráfico de linha (Chart.js), uma série por container, construído
  a partir de um histórico em memória (`deque`, ~10 minutos, amostrado a cada 10s por uma thread
  de fundo que roda independente de alguém ter a tela aberta) — mostra os picos de uso de memória
  dos serviços e, quando pegar a foto certa, dos executores.
- **Logs**: as últimas linhas de `docker logs` do container `oreia-seca` (códigos de cor ANSI
  removidos), a mesma saída que já tem o conteúdo bruto do que o LLM está fazendo quando
  `OREIA_SECA_LLM_DEBUG=1` está ativo — é o "log do chat" em tempo real, sem instrumentação nova.
  Mesmo opt-in do `docker.sock`.

## Como usar

```bash
docker compose up -d forgejo-tls px0
```

Abra **http://localhost:7777**. Sem sessão, cai direto na tela de login — clique **"Entrar com o
Forgejo"**. Se você já estiver logado no Forgejo no mesmo navegador, entra direto (login único de
verdade); na primeira vez, o Forgejo mostra uma tela de consentimento ("autorizar
`oreia-seca-px0`?") — aceite uma vez, e as próximas não pedem nada. Sem OAuth2 configurado (ou
como alternativa), ainda dá pra entrar com usuário + senha/token pessoal no formulário abaixo do
botão. Depois de logado, uma página lista os PRs abertos de `FORGEJO_REPOS` (no `.env`, mesma
lista de `[forgejo].repos`). Clique **"Revisar"** no PR que quiser: ele prepara esse PR (clona a
branch, calcula o merge-base) e te leva pra `/review/`, com o diff aberto — aí é só usar os botões
`Approve`/`Request Changes`/`Merge` da própria interface do px0.

Só uma sessão de PR por vez: clicar em outro PR na lista encerra a atual e abre a nova.

## Autenticação e por que ela também protege o px0

O px0 **não tem login nenhum** — sozinho, quem alcançasse a porta dele teria acesso direto, sem
pedir nada. Por isso ele não é mais exposto diretamente: só a porta do launcher
(`px0/launcher.py`) é publicada no `docker-compose.yml`; o px0 de verdade escuta em `127.0.0.1`,
só alcançável de dentro do próprio container.

O launcher faz **proxy reverso** do px0 sob `/review/` (usando `-base-path /review/`, a mesma
flag que o próprio px0 usa internamente pra isolar sessões — não é um hack) e só deixa passar
quem tiver uma sessão válida. A sessão fica em memória do processo (expira em 12h, ou ao
reiniciar o container).

**Login único (OAuth2) — resolve "já estou logado no Forgejo, por que pede de novo?".** O
launcher registra um app OAuth2 no Forgejo (`oreia-seca-px0`, automatizado em
`make bootstrap`/`scripts/bootstrap.sh`). Entrar manda o navegador pro Forgejo
(`/login/oauth/authorize`); se a sessão dele já existe, ele volta com o código de autorização sem
pedir nada — é o próprio Forgejo que decide se você já está logado, não um cookie nosso tentando
adivinhar isso. A troca do código por um token (e a busca do usuário) é feita servidor-a-servidor
pelo launcher, nunca passa pelo navegador. **Login manual** (usuário + senha/token, validado via
`GET /api/v1/user` com Basic Auth) continua disponível como alternativa, pra quando o OAuth2 não
estiver configurado ou pra acesso não-interativo.

**Allowlist (`PX0_ALLOWED_USERS`).** Só contas do Forgejo nessa lista (separadas por vírgula)
conseguem criar sessão — checado depois de validar a credencial, nos dois caminhos de login. Vazio
= ninguém entra, mesmo padrão do `TELEGRAM_ALLOWED` do bot.

**Importante:** logar aqui só controla o *acesso à ferramenta*. As ações do px0 dentro do
Forgejo (clonar, comentar, aprovar, mesclar) continuam usando o `FORGEJO_OWNER_TOKEN` fixo — não
a conta que você usou pra entrar. Ou seja: qualquer conta na allowlist herda o mesmo nível de
acesso do dono.

## Credenciais

- `FORGEJO_OWNER_TOKEN`: usado pelo px0 pra agir no Forgejo (clonar, comentar, aprovar, mesclar) —
  o mesmo que `/merge`/`/changes` do Telegram já usam.
- `PX0_OAUTH_CLIENT_ID`/`PX0_OAUTH_CLIENT_SECRET`: do app OAuth2 registrado pelo bootstrap — não
  concedem acesso ao Forgejo por si só, só identificam o launcher como app confiável pra pedir
  login.
- `PX0_ALLOWED_USERS`: quem pode entrar (usuários do Forgejo, separados por vírgula).
- Login manual (alternativa ao OAuth2): qualquer usuário/senha (ou usuário/token pessoal) que já
  funcione no Forgejo — ainda precisa estar na allowlist.
- `REST_API_KEY`: reaproveitado pra chamar a API do oreia-seca na tela de **Status**.

## Status (`/status`)

Mostra o que a API REST do oreia-seca (`GET /api/status`, já existente) devolve: fila (rodando,
aguardando, concluídas, falhas), aprovações pendentes, trabalho ativo por `dono/repo#issue`,
papel → modelo, modelos configurados (sem expor chave nenhuma) e agentes. A página se atualiza
sozinha a cada 5s via requisição fetch contra `/status.json` com patch suave do DOM (sem recarregar a tela
ou perder o estado de rolagem) — dando visão em tempo real para acompanhar sem precisar de F5. Não precisa
de `docker.sock` para o status básico do orquestrador (somente para métricas de containers/memória ao vivo).

## Configuração (`/config`) — opt-in, exige `docker.sock`

Edita `oreia-seca.toml` (texto livre, validado com `oreia-seca --check` — rodado via `docker exec`
no container do `oreia-seca` que já está de pé, contra um arquivo candidato em
`volumes/oreia-seca/data/.px0-candidate.toml` — **só grava o arquivo real se a validação passar**)
e `.env` (formulário campo a campo; campos que parecem segredo — nome com `TOKEN`/`KEY`/`SECRET`/
`PASSWORD` — aparecem em branco, e só são trocados se você digitar algo; os outros mostram o
valor real). Salvar reinicia o `oreia-seca` (sempre) e, se foi o `.env`, o próprio `px0` também
(suas variáveis também vêm de lá) — nesse caso a página fica fora do ar por alguns segundos e
você precisa logar de novo depois (reiniciar apaga as sessões, que ficam em memória).

**Isso precisa do `docker.sock` montado no px0** (`docker-compose.override.yml`, mesmo arquivo dos
"containers por executor" — veja `docker-compose.override.example.yml`) **mais os dois arquivos
montados com permissão de escrita**:
```yaml
px0:
  group_add: ["${DOCKER_GID:?...}"]
  volumes:
    - /var/run/docker.sock:/var/run/docker.sock
    - ./oreia-seca.toml:/app-config/oreia-seca.toml
    - ./.env:/app-config/.env
    - ./volumes/oreia-seca/data:/oreia-seca-data
```
Sem isso montado, a página mostra uma mensagem clara em vez de quebrar. **Trade-off de segurança
real, não cosmético** — veja [Segurança](seguranca.md): isso dá ao px0 acesso de leitura/escrita a
TODOS os segredos do projeto (todos os tokens, senha do banco, etc.) e permissão de reiniciar
qualquer container — equivalente a root no host. Só habilite se você realmente quer administrar
pela web em vez do terminal.

## Como isso funciona por dentro

`px0/launcher.py` é o processo principal do container (porta 7777): autentica, lê os PRs abertos
direto da API do Forgejo (Python puro, HTTP simples — sem TLS, isso não tem a ver com o problema
abaixo) e, quando você clica "Revisar", lança o binário real do px0 como processo filho (porta
7778, só em `127.0.0.1`), fazendo proxy reverso sob `/review/`. Isso resolve três problemas que a
documentação do px0 não deixa claras:

### 1. A imagem `ghcr.io/px0-ai/px0` não existe

O guia "Docker & Containers" do próprio px0 usa essa imagem no exemplo — não existe no GHCR
(`docker pull` responde "denied" em qualquer tag, confirmado). A distribuição real é um binário
publicado nos releases do GitHub (`px0-ai/px0`), o mesmo que o `install.sh` oficial baixa.
`px0/Dockerfile` baixa esse binário e constrói a imagem localmente.

### 2. px0 só fala HTTPS com o Gitea/Forgejo — sem opção de desligar

Confirmado testando contra um PR real: trocar o esquema em `gitea.host` não muda nada, o px0
**sempre** monta a URL da API como `https://...`. Não existe chave de configuração pra TLS
("insecure", "skip verify" etc. simplesmente não existem na lista de `gitea.*`). Como o Forgejo
deste projeto é HTTP simples na rede interna do compose, isso bloqueava a integração por completo.

**Solução:** um sidecar `forgejo-tls` (Caddy) faz a ponte — fala HTTPS com o px0 e HTTP puro com o
`forgejo` de verdade. O certificado é gerado uma vez (`tls/ca.crt`, `tls/server.crt`,
`tls/server.key` — gitignorados, regenere se precisar) com SAN `forgejo-tls`, e a CA é instalada
na imagem do px0 em build time (`update-ca-certificates`), então ele confia sem erro de
"unknown authority". Veja `docker-compose.yml` (serviço `forgejo-tls`) e `tls/Caddyfile`.

### 3. OAuth2 do Forgejo pede consentimento, não é "sem fricção nenhuma"

Isso não é um bug a corrigir, é como OAuth2 funciona: na primeira vez que você entra com "Entrar
com o Forgejo", ele mostra uma tela "autorizar `oreia-seca-px0`?" antes de voltar com o código.
Depois dessa vez, enquanto a sessão do Forgejo durar, nem isso aparece — é aí que o login fica
de fato único.

### 4. `git clone` do PR também falhava

O Forgejo devolve o clone URL **público** nos metadados do PR (`localhost:3000` — não existe
dentro do container) e exige autenticação (repo privado). `launcher.py` resolve isso uma vez, no
início do processo, com `git config --global url.insteadOf`, reescrevendo esse host pro serviço
interno `forgejo:3000` já com o token embutido (`http://<token>@forgejo:3000/`) — convenção padrão
do Gitea/Forgejo pra git por HTTP.

### 5. Reiniciar o próprio container no meio da requisição

Pra aplicar o `.env`, o px0 precisa se reiniciar sozinho — mas `docker restart` no próprio
container mata o processo que chamou, então a resposta HTTP nunca sairia se isso rodasse na hora.
A resposta é enviada primeiro; o restart do px0 roda 1,5s depois, num `threading.Timer` separado.
Confirmado no teste real: a chamada demorou mais que os 5s que eu esperei pra conferir na primeira
tentativa (o `docker restart` tem um timeout de parada graciosa antes de matar o processo) —
pareceu ter falhado, mas só precisava de mais alguns segundos.

### 6. Escrever num arquivo bind-mounted sozinho (sem a pasta)

`oreia-seca.toml`/`.env` são bind mounts de um ARQUIVO específico — o diretório pai
(`/app-config`) continua `root:root` por dentro, sem permissão de escrita pro uid do px0, mesmo
com o arquivo em si gravável. O jeito "atômico" normal (escrever num `.tmp` ao lado e renomear)
falha porque CRIAR um arquivo novo exige permissão no diretório, não no arquivo. `launcher.py`
tenta o jeito atômico primeiro e cai pra escrita direta (sobrescrever o arquivo existente, sem
criar nada novo) se isso falhar.

### Regenerar os certificados (se apagar `tls/`)

```bash
MSYS2_ARG_CONV_EXCL="*" openssl req -x509 -new -nodes -key tls/ca.key -sha256 -days 3650 -subj "/CN=OREIA-SECA internal CA" -out tls/ca.crt
openssl genrsa -out tls/server.key 2048
printf "subjectAltName=DNS:forgejo-tls\n" > /tmp/san.ext
MSYS2_ARG_CONV_EXCL="*" openssl req -new -key tls/server.key -subj "/CN=forgejo-tls" -out tls/server.csr
openssl x509 -req -in tls/server.csr -CA tls/ca.crt -CAkey tls/ca.key -CAcreateserial -days 3650 -sha256 -extfile /tmp/san.ext -out tls/server.crt
docker compose build px0 && docker compose up -d forgejo-tls px0
```

(`MSYS2_ARG_CONV_EXCL` só é necessário no Git Bash do Windows, que converte `/CN=...` num caminho
de arquivo.)

## Validado em produção (contra o repo real `minha-org/meu-sistema`)

- Sem sessão: `/` e `/review/` redirecionam pra `/login` — o px0 está de fato inacessível sem login.
- Login manual com usuário `dono` + o próprio `FORGEJO_OWNER_TOKEN` como senha: autentica certo.
- Login errado: 401, sem criar sessão.
- **Allowlist**: com `PX0_ALLOWED_USERS` sem `dono`, o mesmo login válido é recusado (403) — a
  credencial estar certa não basta. Com `dono` na lista, entra normalmente.
- **OAuth2 de ponta a ponta, simulando o navegador de verdade** (não só "deveria funcionar"): login
  real na sessão web do Forgejo via `curl` → `/oauth/login` do launcher → Forgejo reconheceu a
  sessão e pediu consentimento (tela de "autorizar `oreia-seca-px0`?", não pulou direto) →
  submeti o consentimento (`POST /login/oauth/grant`) → Forgejo devolveu o `code` de verdade →
  `/oauth/callback` do launcher trocou por token, buscou o usuário, criou sessão → `/` carregou
  autenticado como `dono`, sem eu ter digitado senha nenhuma nesse trecho.
- Autenticado: a página lista o PR aberto real (título, autor); clicar "Revisar" prepara o PR
  (API via HTTPS pelo sidecar, `git clone` autenticado, merge-base calculado) e o proxy em
  `/review/` devolve a página real do px0 (`<base href="/review/">`, confirmando que
  `-base-path` funcionou).
- Logout revoga a sessão imediatamente — `/review/` volta a redirecionar pra `/login` no mesmo
  navegador, sem precisar reiniciar nada.
- **Status**: `/status` mostra fila/papéis/modelos/agentes reais, puxados do `oreia-seca` já rodando.
- **Configuração**: salvar um `oreia-seca.toml` inválido é recusado (400) e **não** altera o
  arquivo real (confirmado comparando com o conteúdo dentro do container depois); salvar um válido
  valida, grava e reinicia o `oreia-seca` de verdade (confirmei a troca de uptime do container).
  Salvar `.env` reinicia o `oreia-seca` **e** o próprio `px0` — confirmado que o container do px0
  reiniciou de fato (uptime zerou) depois da chamada, e que logar de novo depois funciona normal.

**Não verificado por mim:** o clique real de Approve/Request Changes/Merge na UI do navegador —
isso só dá pra confirmar abrindo de verdade. A API/git já provam que o acesso (leitura e permissão
de push via o token) está correto; falta só você confirmar a experiência visual.
