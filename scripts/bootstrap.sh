#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Prepara o workspace do zero (pode rodar mais de uma vez com segurança):
#   1. cria .env e oreia-seca.toml a partir dos exemplos e gera segredos
#   2. confere se as portas do Forgejo (FORGEJO_HTTP_PORT/FORGEJO_SSH_PORT) estão livres
#   3. sobe Postgres e Forgejo e espera ficarem saudáveis
#   4. cria o seu usuário (dono) e o token dele
#   5. cria uma conta + token para cada agente de [[agents]] no oreia-seca.toml
#   6. cria a organização e os repositórios de [forgejo].repos
#   7. dá acesso de escrita aos agentes, cria as labels, o webhook e
#      protege o branch principal (só entra com a sua aprovação)
#   8. registra o app OAuth2 do px0 (login único com a conta do Forgejo)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

WEBHOOK_URL="${WEBHOOK_URL:-http://oreia-seca:3120/webhooks/forgejo}"
COMPOSE="docker compose"

info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok()   { printf '\033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*"; }
die()  { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

command -v docker >/dev/null || die "docker não encontrado"
command -v curl   >/dev/null || die "curl não encontrado"

# ── 1. Arquivos de configuração ──────────────────────────────────────────────
[[ -f .env ]] || { cp .env.example .env; ok ".env criado"; }
[[ -f oreia-seca.toml ]] || { cp oreia-seca.example.toml oreia-seca.toml; ok "oreia-seca.toml criado (revise os modelos e repos)"; }
mkdir -p volumes/oreia-seca/data volumes/oreia-seca/workspace volumes/oreia-seca/tmp

# ── 1b. Caddyfile e Certificados TLS internos para px0/forgejo-tls ───────────
if [[ ! -f tls/Caddyfile ]]; then
  mkdir -p tls
  cat <<'EOF' > tls/Caddyfile
{
	auto_https off
}

:8443 {
	tls /certs/server.crt /certs/server.key
	reverse_proxy http://forgejo:3000
}
EOF
  ok "tls/Caddyfile criado"
fi

if [[ ! -f tls/ca.crt || ! -f tls/server.crt || ! -f tls/server.key ]]; then
  mkdir -p tls
  info "gerando certificados TLS internos para o sidecar forgejo-tls"
  if command -v openssl >/dev/null 2>&1; then
    MSYS2_ARG_CONV_EXCL="*" openssl req -x509 -new -nodes -newkey rsa:2048 \
      -keyout tls/ca.key -sha256 -days 3650 -subj "/CN=OREIA-SECA internal CA" -out tls/ca.crt 2>/dev/null
    MSYS2_ARG_CONV_EXCL="*" openssl req -new -nodes -newkey rsa:2048 \
      -keyout tls/server.key -subj "/CN=forgejo-tls" -out tls/server.csr 2>/dev/null
    printf "subjectAltName=DNS:forgejo-tls\n" > tls/san.ext
    openssl x509 -req -in tls/server.csr -CA tls/ca.crt -CAkey tls/ca.key -CAcreateserial -days 3650 -sha256 \
      -extfile tls/san.ext -out tls/server.crt 2>/dev/null
    rm -f tls/server.csr tls/san.ext tls/ca.srl 2>/dev/null || true
  else
    # Opção 1: Via container Docker alpine/openssl (dispensa OpenSSL no host)
    docker run --rm -v "${PWD}:/work" -w /work alpine/openssl sh -c "\
      openssl req -x509 -new -nodes -newkey rsa:2048 -keyout tls/ca.key -sha256 -days 3650 -subj '/CN=OREIA-SECA internal CA' -out tls/ca.crt && \
      openssl req -new -nodes -newkey rsa:2048 -keyout tls/server.key -subj '/CN=forgejo-tls' -out tls/server.csr && \
      printf 'subjectAltName=DNS:forgejo-tls\n' > tls/san.ext && \
      openssl x509 -req -in tls/server.csr -CA tls/ca.crt -CAkey tls/ca.key -CAcreateserial -days 3650 -sha256 -extfile tls/san.ext -out tls/server.crt && \
      rm -f tls/server.csr tls/san.ext tls/ca.srl" 2>/dev/null
  fi
  ok "certificados TLS gerados em tls/"
fi

get_env() { grep -E "^$1=" .env | head -1 | cut -d= -f2- || true; }
set_env() {
  local key="$1" val="$2"
  if grep -qE "^$key=" .env; then
    local tmp; tmp="$(mktemp)"
    awk -v k="$key" -v v="$val" 'BEGIN{FS=OFS="="} $1==k {$0=k"="v} {print}' .env > "$tmp" && mv "$tmp" .env
  else
    echo "$key=$val" >> .env
  fi
}
gen_secret() {
  local bytes="${1:-24}"
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex "$bytes"
  elif [[ -r /dev/urandom ]]; then
    head -c "$bytes" /dev/urandom | od -An -tx1 | tr -d ' \n'
  else
    docker run --rm alpine/openssl rand -hex "$bytes" 2>/dev/null | tr -d '\r\n'
  fi
}
ensure_secret() { [[ -n "$(get_env "$1")" ]] || { set_env "$1" "$(gen_secret 24)"; ok "$1 gerado"; }; }

ensure_secret FORGEJO_DB_PASSWORD
ensure_secret FORGEJO_WEBHOOK_SECRET
ensure_secret REST_API_KEY
ensure_secret FORGEJO_OWNER_PASSWORD

OWNER="$(get_env FORGEJO_OWNER_USER)";   OWNER="${OWNER:-dono}"
OWNER_EMAIL="$(get_env FORGEJO_OWNER_EMAIL)"; OWNER_EMAIL="${OWNER_EMAIL:-$OWNER@oreia-seca.local}"

HTTP_PORT="$(get_env FORGEJO_HTTP_PORT)"; HTTP_PORT="${HTTP_PORT:-3000}"
SSH_PORT="$(get_env FORGEJO_SSH_PORT)";   SSH_PORT="${SSH_PORT:-2222}"
FORGEJO_URL="${FORGEJO_URL:-http://localhost:$HTTP_PORT}"

# ── 2. Portas livres? ────────────────────────────────────────────────────────
# Só checa se o nosso próprio Forgejo ainda não está rodando — reexecutar o
# bootstrap com tudo já no ar não deve acusar "porta ocupada" por si mesmo.
port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3>&- 3<&- 2>/dev/null; return 0; }; return 1; }
service_running() { [[ -n "$($COMPOSE ps -q --status running "$1" 2>/dev/null)" ]]; }

if ! service_running forgejo; then
  if port_open "$HTTP_PORT"; then
    die "porta $HTTP_PORT (HTTP do Forgejo) já está em uso por outro processo/container. Mude FORGEJO_HTTP_PORT no .env e rode de novo, ou pare o que estiver usando essa porta."
  fi
  if port_open "$SSH_PORT"; then
    die "porta $SSH_PORT (SSH do Forgejo) já está em uso por outro processo/container. Mude FORGEJO_SSH_PORT no .env e rode de novo, ou pare o que estiver usando essa porta."
  fi
fi

# ── 3. Forgejo no ar ─────────────────────────────────────────────────────────
info "subindo postgres e forgejo"
$COMPOSE up -d postgres forgejo
info "aguardando o Forgejo responder em $FORGEJO_URL"
for _ in $(seq 1 60); do
  curl -fsS "$FORGEJO_URL/api/healthz" >/dev/null 2>&1 && break
  sleep 2
done
curl -fsS "$FORGEJO_URL/api/healthz" >/dev/null 2>&1 || die "Forgejo não respondeu"
ok "Forgejo saudável"

fj() { $COMPOSE exec -T -u git forgejo forgejo "$@"; }

create_user() { # usuario email senha admin(sim/não)
  local extra=()
  [[ "$4" == "sim" ]] && extra+=(--admin)
  if fj admin user list 2>/dev/null | awk '{print $2}' | grep -qx "$1"; then
    ok "usuário $1 já existe"
  else
    fj admin user create --username "$1" --email "$2" --password "$3" --must-change-password=false "${extra[@]}" >/dev/null
    ok "usuário $1 criado"
  fi
}

new_token() { # usuario nome escopos
  fj admin user generate-access-token --username "$1" --token-name "$2-$(date +%s)" --scopes "$3" --raw | tail -1 | tr -d '\r'
}

# ── 4. Dono ──────────────────────────────────────────────────────────────────
create_user "$OWNER" "$OWNER_EMAIL" "$(get_env FORGEJO_OWNER_PASSWORD)" sim
if [[ -z "$(get_env FORGEJO_OWNER_TOKEN)" ]]; then
  set_env FORGEJO_OWNER_TOKEN "$(new_token "$OWNER" oreia-seca-dono all)"
  ok "token do dono gerado"
fi
OWNER_TOKEN="$(get_env FORGEJO_OWNER_TOKEN)"

api() { # método caminho [json]
  local method="$1" path="$2" data="${3:-}"
  local args=(-sS -o /tmp/oreia-seca-api.out -w '%{http_code}' -X "$method" "$FORGEJO_URL/api/v1$path"
              -H "Authorization: token $OWNER_TOKEN" -H "Content-Type: application/json")
  [[ -n "$data" ]] && args+=(-d "$data")
  curl "${args[@]}"
}
api_ok() { # descrição método caminho [json] — aceita 2xx e 409/422 (já existe)
  local code; code="$(api "$2" "$3" "${4:-}")"
  case "$code" in
    2??) ok "$1" ;;
    409|422) ok "$1 (já existia)" ;;
    *) warn "$1 falhou (HTTP $code): $(head -c 300 /tmp/oreia-seca-api.out)" ;;
  esac
}

# ── 4b. App OAuth2 do px0 (login único com a conta do Forgejo) ──────────────
# Sem isso, o px0 cai no formulário usuário+senha/token — funciona, mas pede
# login de novo mesmo com sessão ativa no Forgejo. Veja docs/px0.md.
if [[ -z "$(get_env PX0_OAUTH_CLIENT_ID)" ]]; then
  PX0_PORT="$(get_env PX0_PORT)"; PX0_PORT="${PX0_PORT:-7777}"
  code="$(api POST /user/applications/oauth2 \
    "{\"name\":\"oreia-seca-px0\",\"confidential_client\":true,\"redirect_uris\":[\"http://localhost:$PX0_PORT/oauth/callback\"]}")"
  if [[ "$code" == 2?? ]]; then
    client_id="$(grep -oE '"client_id":"[^"]*"' /tmp/oreia-seca-api.out | head -1 | cut -d'"' -f4)"
    client_secret="$(grep -oE '"client_secret":"[^"]*"' /tmp/oreia-seca-api.out | head -1 | cut -d'"' -f4)"
    if [[ -n "$client_id" && -n "$client_secret" ]]; then
      set_env PX0_OAUTH_CLIENT_ID "$client_id"
      set_env PX0_OAUTH_CLIENT_SECRET "$client_secret"
      ok "app OAuth2 do px0 registrado"
    else
      warn "app OAuth2 do px0: resposta sem client_id/client_secret, pulando"
    fi
  else
    warn "app OAuth2 do px0 falhou (HTTP $code): $(head -c 300 /tmp/oreia-seca-api.out)"
  fi
fi

# ── 5. Agentes (lidos do oreia-seca.toml) ────────────────────────────────────
# Extrai "user token_env label" de cada bloco [[agents]]
mapfile -t AGENTS < <(awk '
  /^\[\[agents\]\]/ { if (u) print u, t, l; u=t=l=""; inside=1; next }
  /^\[/ { if (inside && u) print u, t, l; inside=0; u=t=l=""; next }
  inside && /^user[[:space:]]*=/      { gsub(/.*=[[:space:]]*"|".*/, ""); u=$0 }
  inside && /^token_env[[:space:]]*=/ { gsub(/.*=[[:space:]]*"|".*/, ""); t=$0 }
  inside && /^label[[:space:]]*=/     { gsub(/.*=[[:space:]]*"|".*/, ""); l=$0 }
  END { if (inside && u) print u, t, l }
' oreia-seca.toml)

for line in "${AGENTS[@]}"; do
  read -r user token_env _label <<<"$line"
  create_user "$user" "$user@agents.oreia-seca.local" "$(gen_secret 16)" não
  if [[ -z "$(get_env "$token_env")" ]]; then
    set_env "$token_env" "$(new_token "$user" oreia-seca write:repository,write:issue,read:user)"
    ok "token de $user salvo em $token_env"
  fi
done

# ── 6/7. Repositórios ────────────────────────────────────────────────────────
REPOS="$(awk '/^\[forgejo\]/{f=1;next} /^\[/{f=0} f && /^repos[[:space:]]*=/' oreia-seca.toml | grep -oE '"[^"]+/[^"]+"' | tr -d '"' || true)"
[[ -n "$REPOS" ]] || warn "nenhum repositório em [forgejo].repos"

# Mantém FORGEJO_REPOS no .env sincronizado com oreia-seca.toml (necessário para o px0)
if [[ -n "$REPOS" ]]; then
  repos_csv="$(echo "$REPOS" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')"
  set_env FORGEJO_REPOS "$repos_csv"
fi

COLORS=(1f6feb 8250df 2da44e bf8700 cf222e)
SECRET="$(get_env FORGEJO_WEBHOOK_SECRET)"

for full in $REPOS; do
  org="${full%%/*}"; repo="${full#*/}"
  if [[ "$org" != "$OWNER" ]]; then
    api_ok "organização $org" POST /orgs "{\"username\":\"$org\",\"visibility\":\"private\"}"
    api_ok "repositório $full" POST "/orgs/$org/repos" "{\"name\":\"$repo\",\"auto_init\":true,\"default_branch\":\"main\",\"private\":true}"
  else
    api_ok "repositório $full" POST /user/repos "{\"name\":\"$repo\",\"auto_init\":true,\"default_branch\":\"main\",\"private\":true}"
  fi

  i=0
  for line in "${AGENTS[@]}"; do
    read -r user _token label <<<"$line"
    api_ok "acesso de escrita para $user em $full" PUT "/repos/$org/$repo/collaborators/$user" '{"permission":"write"}'
    api_ok "label $label" POST "/repos/$org/$repo/labels" "{\"name\":\"$label\",\"color\":\"#${COLORS[$((i % ${#COLORS[@]}))]}\"}"
    i=$((i + 1))
  done

  # Webhook para o orquestrador (evita duplicar)
  if api GET "/repos/$org/$repo/hooks" >/dev/null && grep -q "$WEBHOOK_URL" /tmp/oreia-seca-api.out; then
    ok "webhook em $full (já existia)"
  else
    api_ok "webhook em $full" POST "/repos/$org/$repo/hooks" "{
      \"type\":\"forgejo\",\"active\":true,
      \"events\":[\"issues\",\"issue_label\",\"issue_comment\",\"pull_request\",\"pull_request_comment\"],
      \"config\":{\"url\":\"$WEBHOOK_URL\",\"content_type\":\"json\",\"secret\":\"$SECRET\"}}"
  fi

  # main protegido: ninguém faz push direto; merge exige 1 aprovação sua
  api_ok "proteção do branch main em $full" POST "/repos/$org/$repo/branch_protections" "{
    \"rule_name\":\"main\",\"enable_push\":false,
    \"required_approvals\":1,\"enable_approvals_whitelist\":true,
    \"approvals_whitelist_username\":[\"$OWNER\"],
    \"block_on_rejected_reviews\":true,\"dismiss_stale_approvals\":true}"
done

echo
ok "pronto!"
echo "   Forgejo:  $FORGEJO_URL  (usuário $OWNER, senha em FORGEJO_OWNER_PASSWORD no .env)"
echo "   Próximos: make llm-setup (ou ./oreia llm-setup, sem make) para um LLM externo, depois make up / ./oreia up"
