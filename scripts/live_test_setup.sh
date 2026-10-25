#!/usr/bin/env bash
# Instala aider/opencode/goose/codex num container de teste descartável (sem
# Rust/CLIs instalados no host) para rodar o teste ao vivo dos executores
# externos contra uma API real. Não precisa disso para rodar o projeto — só
# para revalidar os presets em docs/executores.md/BUILTIN_PRESETS depois de
# alguma mudança nessas ferramentas.
#
# Uso (dentro de rust:1-bookworm, com o repo montado em /src):
#   bash /src/scripts/live_test_setup.sh
#   OREIA_SECA_LIVE_EXTERNAL_TESTS=1 OPENROUTER_KEY=<sua chave> \
#     cargo test -p oreia-seca live_presets_actually_work -- --nocapture
set -e
apt-get update -qq
apt-get install -y -qq --no-install-recommends git python3 python3-pip nodejs npm bzip2 curl ca-certificates >/dev/null
pip install --no-cache-dir --break-system-packages --quiet aider-chat
curl -fsSL https://opencode.ai/install | bash >/dev/null 2>&1
cp /root/.opencode/bin/opencode /usr/local/bin/opencode
curl -fsSL -o /tmp/goose.tar.bz2 https://github.com/block/goose/releases/latest/download/goose-x86_64-unknown-linux-gnu.tar.bz2
mkdir -p /tmp/goose-extract && tar xjf /tmp/goose.tar.bz2 -C /tmp/goose-extract
mv /tmp/goose-extract/goose /usr/local/bin/goose && chmod +x /usr/local/bin/goose
npm install -g @openai/codex --silent >/dev/null 2>&1
echo "TOOLS_READY"
aider --version
opencode --version
goose --version
codex --version
