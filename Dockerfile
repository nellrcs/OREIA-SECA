# ── Build ────────────────────────────────────────────────────────────────────
FROM rust:1-bookworm AS build
WORKDIR /src

# Cache de dependências: compila só os manifests primeiro
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates/oreia-core/Cargo.toml crates/oreia-core/
COPY crates/oreia-llm/Cargo.toml crates/oreia-llm/
COPY crates/oreia-forgejo/Cargo.toml crates/oreia-forgejo/
COPY crates/oreia-channels/Cargo.toml crates/oreia-channels/
COPY crates/oreia-seca/Cargo.toml crates/oreia-seca/
RUN for c in oreia-core oreia-llm oreia-forgejo oreia-channels; do \
      mkdir -p crates/$c/src && echo "" > crates/$c/src/lib.rs; done \
 && mkdir -p crates/oreia-seca/src && echo "fn main() {}" > crates/oreia-seca/src/main.rs \
 && cargo build --release --locked \
 && rm -rf crates/*/src

COPY crates crates
RUN touch crates/*/src/*.rs && cargo build --release --locked

# ── Runtime ──────────────────────────────────────────────────────────────────
# Os agentes rodam comandos (build, testes) DENTRO deste container.
# Adicione aqui as ferramentas da stack do seu sistema (node, rustup, go...).
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git curl python3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 oreia-seca

# Cliente docker (só o binário estático, sem dockerd) — necessário pros
# "containers por executor" (opencode/codex rodando em imagem própria via
# `docker run --rm`, não dentro deste container). Precisa de
# /var/run/docker.sock montado (ver docker-compose.override.example.yml) e de
# OREIA_SECA_HOST_DIR no .env. Sem isso configurado, este binário fica sem uso
# — nada quebra, nenhum outro executor (builtin, ou um externo sem `container`
# no `[executors.*]`) depende dele. Veja docs/executores.md#containers-por-executor.
RUN curl -fsSL https://download.docker.com/linux/static/stable/x86_64/docker-27.3.1.tgz \
    -o /tmp/docker.tgz \
 && tar xzf /tmp/docker.tgz -C /tmp \
 && mv /tmp/docker/docker /usr/local/bin/docker \
 && rm -rf /tmp/docker.tgz /tmp/docker

# ── Executores externos locais (opcional) ───────────────────────────────────
# Rodar uma CLI de terceiros DENTRO deste container, em vez de containerizada
# (o padrão deste projeto desde que opencode/codex passaram a rodar em imagem
# própria — veja executors/*.Dockerfile). Só faz sentido pra quem não quer
# lidar com Docker-outside-of-Docker: configure `[executors.<nome>]` SEM
# `container` e descomente o bloco correspondente aqui. Os quatro blocos
# abaixo foram verificados de verdade contra uma API real em 2026-10-25 — não
# são só uma leitura do --help; veja docs/executores.md para o comando final
# de cada um e `scripts/live_test_setup.sh` para revalidar depois de uma
# atualização.

# OpenCode.
# O instalador grava em ~/.opencode/bin e só ajusta o PATH via .bashrc (de
# root) — inútil tanto porque o executor roda com `sh -c` (não lê .bashrc)
# quanto porque o processo final roda como "oreia-seca", não "root", e /root
# é 700 (sem acesso nenhum pra outro usuário). Por isso o `cp`: copia pra
# /usr/local/bin, que todo usuário do container alcança.
# RUN curl -fsSL https://opencode.ai/install | bash \
#  && cp /root/.opencode/bin/opencode /usr/local/bin/opencode

# Aider (pip). Debian Bookworm recusa instalar fora de venv (PEP 668) sem
# --break-system-packages — aceitável aqui porque o container é de uso único.
# Testado em 2026-10-25 e descartado deste projeto: o preset atual destruiu o
# arquivo editado (substituiu o conteúdo inteiro em vez de só a linha pedida),
# a verificação ("algum arquivo mudou") não pega esse tipo de erro. Não habilite
# sem investigar a causa raiz primeiro.
# RUN apt-get update && apt-get install -y --no-install-recommends python3-pip \
#  && rm -rf /var/lib/apt/lists/* \
#  && pip install --no-cache-dir --break-system-packages aider-chat

# Goose. O instalador oficial ("curl | bash") tem a mesma armadilha de
# permissão do opencode E tenta um passo de configuração interativa que falha
# sem terminal ("/dev/tty: No such device or address") — não serve pra build
# não interativo. Em vez dele, baixa e extrai o binário direto.
# Testado em 2026-10-25 e descartado deste projeto: resultado correto, mas
# ~3-5x mais lento que os outros executores na mesma tarefa.
# RUN apt-get update && apt-get install -y --no-install-recommends bzip2 \
#  && rm -rf /var/lib/apt/lists/* \
#  && curl -fsSL -o /tmp/goose.tar.bz2 \
#       https://github.com/block/goose/releases/latest/download/goose-x86_64-unknown-linux-gnu.tar.bz2 \
#  && mkdir -p /tmp/goose-extract && tar xjf /tmp/goose.tar.bz2 -C /tmp/goose-extract \
#  && mv /tmp/goose-extract/goose /usr/local/bin/goose && chmod +x /usr/local/bin/goose \
#  && rm -rf /tmp/goose.tar.bz2 /tmp/goose-extract

# Codex. Exige node/npm, que não vêm na imagem base.
# RUN apt-get update && apt-get install -y --no-install-recommends nodejs npm \
#  && rm -rf /var/lib/apt/lists/* \
#  && npm install -g @openai/codex

WORKDIR /app
COPY --from=build /src/target/release/oreia-seca /usr/local/bin/oreia-seca
COPY oreia-seca.example.toml /app/oreia-seca.example.toml
COPY agents /app/agents
COPY skills /app/skills

RUN mkdir -p /app/data /app/workspace && chown -R oreia-seca:oreia-seca /app
USER oreia-seca

ENV RUST_LOG=info
EXPOSE 3120
HEALTHCHECK --interval=30s --timeout=3s CMD curl -fsS http://localhost:3120/health || exit 1
ENTRYPOINT ["oreia-seca"]
CMD ["--config", "/app/oreia-seca.toml"]
