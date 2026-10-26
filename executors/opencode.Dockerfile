# Imagem isolada para o executor "opencode" — roda via `docker run --rm` lançado
# pelo oreia-seca (containers por executor, veja docs/executores.md#containers-por-executor),
# não como processo dentro do container do orquestrador. uid/gid 10001 batem com
# o usuário "oreia-seca" do Dockerfile principal: os volumes montados em tempo de
# execução (/workspace, o repositório clonado; /scratch, a instrução da fase +
# o opencode.jsonc ad-hoc) são do host, criados por aquele uid — este Dockerfile
# só precisa enxergar o mesmo NÚMERO, não o mesmo nome de usuário.
#
# Build: make executors-build (ou `docker build -f executors/opencode.Dockerfile
# -t oreia-seca-exec-opencode .` direto, a partir da raiz do repo).
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 oreiaseca

# Mesma pegadinha do Dockerfile principal: o instalador grava em ~/.opencode/bin
# (de root, durante o build) — copia pra /usr/local/bin, que qualquer usuário alcança.
RUN curl -fsSL https://opencode.ai/install | bash \
 && cp /root/.opencode/bin/opencode /usr/local/bin/opencode

USER oreiaseca
WORKDIR /workspace
