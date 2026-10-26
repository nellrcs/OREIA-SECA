# px0 (https://px0.ai/docs/) não publica imagem Docker de verdade — o guia
# "Docker & Containers" da própria documentação cita `ghcr.io/px0-ai/px0:latest`,
# mas esse pacote não existe no GHCR (confirmado: `docker pull` responde "denied"
# tanto em :latest quanto em tags de versão). A distribuição real é um binário
# único publicado nos releases do GitHub (é o que o install.sh deles baixa) —
# então empacotamos esse binário aqui, do mesmo jeito que o instalador oficial.
FROM debian:bookworm-slim

ARG PX0_VERSION=0.1.20

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 px0

# CA interna só pra falar HTTPS com o sidecar Caddy (tls/, veja docs/px0.md) na
# frente do Forgejo — px0 exige HTTPS e não tem opção de pular verificação de
# certificado. Sem isto: "certificate signed by unknown authority".
COPY tls/ca.crt /usr/local/share/ca-certificates/oreia-seca-internal-ca.crt
RUN update-ca-certificates \
    && curl -fsSL -o /usr/local/bin/px0 \
        "https://github.com/px0-ai/px0/releases/download/v${PX0_VERSION}/px0-${PX0_VERSION}-linux-amd64" \
    && chmod +x /usr/local/bin/px0

# Cliente docker (só o binário estático, sem dockerd) — só tem efeito com
# /var/run/docker.sock montado (docker-compose.override.yml, opt-in, mesmo
# trade-off de segurança dos "containers por executor" — ver docs/seguranca.md).
# Usado pela tela de configuração (restart do oreia-seca após salvar, validação
# via `docker exec` no container já rodando). Sem o socket montado, esse
# binário fica sem uso — a tela de PRs continua funcionando normalmente.
RUN curl -fsSL https://download.docker.com/linux/static/stable/x86_64/docker-27.3.1.tgz \
    -o /tmp/docker.tgz \
 && tar xzf /tmp/docker.tgz -C /tmp \
 && mv /tmp/docker/docker /usr/local/bin/docker \
 && rm -rf /tmp/docker.tgz /tmp/docker

# launcher.py é o processo principal: serve a página de PRs (porta 7777) e, por
# baixo, lança o px0 de verdade (binário acima) pra cada PR que você clicar.
COPY px0/launcher.py /usr/local/bin/px0-launcher.py

USER px0
ENV HOME=/home/px0
WORKDIR /workspace
ENTRYPOINT ["python3", "/usr/local/bin/px0-launcher.py"]
