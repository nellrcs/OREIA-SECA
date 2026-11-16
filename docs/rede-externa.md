# Rede Docker externa

Se você já tem algo nesta máquina que precisa falar diretamente com o Forgejo, o px0 ou o
oreia-seca — um `cloudflared` (Cloudflare Tunnel), Tailscale, um Nginx/Traefik que já roda em
outro projeto — não publique as portas desses serviços (`-p host:container`) pra internet: anexe
os containers do OREIA-SECA à mesma rede Docker **externa** que essa outra coisa já usa, e aponte
pro **nome do serviço** (DNS interno do Docker), não para `localhost`.

## Por quê

Dentro de uma rede Docker, os containers se enxergam pelo nome do serviço (`forgejo`, `px0`,
`oreia-seca`), não por `localhost` nem pelo IP da máquina. Quem está do outro lado (cloudflared,
proxy, etc.) só consegue chegar em `http://forgejo:3000` se estiver **na mesma rede Docker**. Como
este projeto e o seu outro stack são `docker compose` separados, cada um cria sua própria rede por
padrão — por isso a rede precisa ser **externa** (existe fora deste projeto, compartilhada entre
os dois).

## Como configurar

1. Descubra ou crie o nome da rede Docker compartilhada. Se o outro stack já tem uma rede nomeada
   (`docker network ls`), use o nome dela; senão, crie uma e conecte os dois lados a ela:

   ```bash
   docker network create minha-rede-externa
   ```

   (o nome é livre — `rede-cloudflare`, `minha-rede-externa`, o que fizer sentido pra você — e
   conecte o outro serviço, ex. o `cloudflared`, a ela no `docker-compose.yml` dele, em
   `networks:`, se ainda não estiver.)

2. No `.env` deste projeto:

   ```bash
   EXTERNAL_NETWORK=minha-rede-externa
   ```

3. Copie [`docker-compose.external-network.example.yml`](../docker-compose.external-network.example.yml)
   para dentro do seu `docker-compose.override.yml` (crie-o se não existir — já está no
   `.gitignore`, fica só nesta máquina). Se você já usa o override pra
   [executores por container](executores.md#containers-por-executor), mescle as duas seções
   `services:`/`networks:` num só arquivo.

4. Suba de novo:

   ```bash
   docker compose up -d --force-recreate forgejo px0 oreia-seca
   ```

5. Na outra ponta, aponte cada destino pro serviço interno — por exemplo, num `config.yml` de
   `cloudflared` ou no painel do Cloudflare Zero Trust:

   | Hostname/destino público | Endereço interno (mesma rede Docker) |
   |---|---|
   | `git.seudominio.com` | `http://forgejo:3000` |
   | `review.seudominio.com` | `http://px0:7777` |
   | `oreia.seudominio.com` | `http://oreia-seca:3120` |

   Anexe só os serviços que você realmente quer expor — comente os demais no
   `docker-compose.external-network.example.yml` antes de copiar.

## Atenção

- Se o destino final é a internet pública (Cloudflare Tunnel, etc.), `FORGEJO_PUBLIC_URL` e
  `PX0_PUBLIC_URL` (no `.env`) precisam ser o hostname público
  (`https://git.seudominio.com/`, `https://review.seudominio.com`), não `localhost` — mesmo
  motivo do acesso pela rede local, ver [px0 → Acesso pela rede local](px0.md#acesso-pela-rede-local).
  Sem isso, o login "Entrar com o Forgejo" do px0 e o OAuth2 redirecionam pra `localhost`, que não
  existe para quem acessa de fora.
- Isso só troca a rede, não a autenticação: px0 continua exigindo login (ver
  [Segurança](seguranca.md)), e o Forgejo continua com `DISABLE_REGISTRATION=true`. Exposto à
  internet por um túnel/proxy, considere também uma camada de autenticação na frente (ex. Access
  Policy do Cloudflare Zero Trust), já que a rede por si só não autentica ninguém.
- Não anexe o `forgejo-tls` (sidecar interno do px0) nem o `postgres` à rede externa — não têm
  porta pensada pra tráfego de fora e não precisam disso para o px0 funcionar.
