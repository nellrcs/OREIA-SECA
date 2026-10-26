# Imagem isolada para o executor "codex" — ver cabeçalho de
# executors/opencode.Dockerfile (mesmo padrão: container por executor, uid/gid
# 10001 batendo com o usuário "oreia-seca" do Dockerfile principal).
#
# Build: make executors-build (ou `docker build -f executors/codex.Dockerfile
# -t oreia-seca-exec-codex .` direto, a partir da raiz do repo).
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git curl nodejs npm \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 oreiaseca \
 && npm install -g @openai/codex

USER oreiaseca
WORKDIR /workspace
