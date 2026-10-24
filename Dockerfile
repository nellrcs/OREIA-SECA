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

# ── Executores externos (opcional) ──────────────────────────────────────────
# Nenhum vem instalado por padrão — só o executor "builtin" (ações embutidas)
# funciona de cara. Para usar um degrau de escalada com Aider, OpenCode, Goose
# ou Codex (veja docs/executores.md), descomente a linha correspondente.
#
# RUN pip install --no-cache-dir aider-chat
# RUN curl -fsSL https://opencode.ai/install | bash
# RUN curl -fsSL https://github.com/block/goose/releases/latest/download/download_cli.sh | bash
# RUN npm install -g @openai/codex   # exige node, adicione-o acima se for usar

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
