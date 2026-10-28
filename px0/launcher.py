#!/usr/bin/env python3
"""Página intermediária: autentica com as MESMAS credenciais do Forgejo (conta
ou token pessoal), lista PRs abertos, e ao clicar "Revisar" lança o px0 pra
esse PR e faz proxy reverso dele sob a MESMA sessão — o px0 em si não tem
login nenhum, então é essa camada que o protege. Veja docs/px0.md.

Só usa a biblioteca padrão de propósito (sem dependências pra instalar).
"""
import base64
import calendar
import collections
import html
import http.client
import json
import os
import re
import secrets
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request
from http import cookies
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, urlencode, urlparse

FORGEJO_API = os.environ.get("FORGEJO_API", "http://forgejo:3000/api/v1")
FORGEJO_INTERNAL = "http://forgejo:3000"  # chamadas servidor-a-servidor (token exchange, userinfo)
GITEA_TOKEN = os.environ.get("GITEA_TOKEN", "")
REPOS = [r.strip() for r in os.environ.get("FORGEJO_REPOS", "").split(",") if r.strip()]
FORGEJO_ROOT_URL = os.environ.get("FORGEJO_ROOT_URL", "")  # URL pública — pro NAVEGADOR navegar até lá
PX0_PUBLIC_URL = os.environ.get("PX0_PUBLIC_URL", "http://localhost:7777")
OAUTH_CLIENT_ID = os.environ.get("PX0_OAUTH_CLIENT_ID", "")
OAUTH_CLIENT_SECRET = os.environ.get("PX0_OAUTH_CLIENT_SECRET", "")
OAUTH_REDIRECT_URI = f"{PX0_PUBLIC_URL}/oauth/callback"
ALLOWED_USERS = [u.strip() for u in os.environ.get("PX0_ALLOWED_USERS", "").split(",") if u.strip()]
REVIEW_PORT = int(os.environ.get("PX0_REVIEW_PORT", "7778"))
LAUNCHER_PORT = int(os.environ.get("PX0_LAUNCHER_PORT", "7777"))
BASE_PATH = "/review/"

OREIA_SECA_API = os.environ.get("OREIA_SECA_API", "http://oreia-seca:3120")
REST_API_KEY = os.environ.get("REST_API_KEY", "")

# Edição de configuração (opt-in — exige docker-compose.override.yml com
# /var/run/docker.sock e estes dois arquivos montados; veja docs/px0.md).
# A validação do candidato de TOML não lê mais um arquivo em disco: vai
# direto pro POST /api/check do oreia-seca já rodando (ver check_candidate),
# que também roda as checagens de rede/processo do que mudaria.
TOML_PATH = "/app-config/oreia-seca.toml"
ENV_PATH = "/app-config/.env"
# Mesmo arquivo que data/escalation.jsonl (usado pelo /stats do Telegram) —
# cada linha é uma tentativa de fase: papel, degrau, executor, modelo, se
# passou e quanto demorou. É o "log do que está sendo feito" + "qual
# executor" + "tempo de execução" da tela de Status. Também opt-in (mesmo
# mount de /oreia-seca-data da edição de configuração).
ESCALATION_LOG_PATH = "/oreia-seca-data/escalation.jsonl"
COMPOSE_PROJECT = "oreia-seca"
SECRET_KEY_RE = re.compile(r"(TOKEN|KEY|SECRET|PASSWORD)", re.IGNORECASE)

SESSION_COOKIE = "px0_session"
SESSION_TTL = 12 * 3600
sessions = {}  # sid -> {"user": str, "expires": float} — em memória; reinicia com o container

OAUTH_STATE_TTL = 300
oauth_states = {}  # state -> expiry — só durante o round-trip do /oauth/login ao /oauth/callback


def is_allowed(user):
    """Mesmo padrão do TELEGRAM_ALLOWED (oreia-channels/telegram.rs): lista
    vazia = ninguém entra, mesmo com credencial Forgejo válida."""
    return user in ALLOWED_USERS


# Sessão de PR única por vez (mesma porta interna reaproveitada) — simples o
# bastante pro uso real: revisar PRs um por vez. Abrir outro PR encerra o anterior.
current = {"proc": None, "key": None}


def setup_git_rewrite():
    """O Forgejo devolve o clone URL PÚBLICO nos metadados do PR (ex.
    localhost:3000) — não existe dentro deste container. Reescreve pro serviço
    interno, já com o token embutido (convenção Gitea/Forgejo pra git por
    HTTP). Roda uma vez, no início do processo."""
    if not FORGEJO_ROOT_URL:
        return
    hostport = FORGEJO_ROOT_URL.split("://", 1)[-1].rstrip("/")
    target = f"http://{GITEA_TOKEN}@forgejo:3000/"
    for scheme in ("http", "https"):
        subprocess.run(
            ["git", "config", "--global", "--add", f"url.{target}.insteadOf", f"{scheme}://{hostport}/"],
            check=True,
        )


# ── Autenticação (mesma conta/token do Forgejo) ──────────────────────────────


def check_forgejo_login(username, password):
    """Valida usuário+senha (ou usuário+token pessoal) direto na API do
    Forgejo — a mesma conta que você já usa pra entrar nele. Não dá pra saber
    de fora se é senha ou token; o Forgejo aceita os dois em Basic Auth."""
    basic = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req = urllib.request.Request(f"{FORGEJO_API}/user", headers={"Authorization": f"Basic {basic}"})
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            data = json.load(r)
            return data.get("login") or username
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError):
        return None


def oauth_authorize_url(state):
    root = FORGEJO_ROOT_URL.rstrip("/")
    q = urlencode(
        {
            "client_id": OAUTH_CLIENT_ID,
            "redirect_uri": OAUTH_REDIRECT_URI,
            "response_type": "code",
            "state": state,
        }
    )
    return f"{root}/login/oauth/authorize?{q}"


def oauth_exchange_code(code):
    """Troca o code por um access token e devolve o usuário logado — chamada
    servidor-a-servidor, por isso usa o host INTERNO (igual toda chamada à
    API), não a URL pública que o navegador usou."""
    body = json.dumps(
        {
            "client_id": OAUTH_CLIENT_ID,
            "client_secret": OAUTH_CLIENT_SECRET,
            "code": code,
            "grant_type": "authorization_code",
            "redirect_uri": OAUTH_REDIRECT_URI,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        f"{FORGEJO_INTERNAL}/login/oauth/access_token",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=10) as r:
        access_token = json.load(r)["access_token"]
    req = urllib.request.Request(
        f"{FORGEJO_INTERNAL}/api/v1/user", headers={"Authorization": f"Bearer {access_token}"}
    )
    with urllib.request.urlopen(req, timeout=10) as r:
        data = json.load(r)
        return data.get("login")


def new_session(user):
    sid = secrets.token_urlsafe(32)
    csrf = secrets.token_urlsafe(32)
    sessions[sid] = {"user": user, "expires": time.time() + SESSION_TTL, "csrf": csrf}
    return sid


def session_from_cookie(handler):
    raw = handler.headers.get("Cookie", "")
    jar = cookies.SimpleCookie()
    jar.load(raw)
    if SESSION_COOKIE not in jar:
        return None
    sid = jar[SESSION_COOKIE].value
    s = sessions.get(sid)
    if not s or s["expires"] < time.time():
        sessions.pop(sid, None)
        return None
    if "csrf" not in s:
        s["csrf"] = secrets.token_urlsafe(32)
    return s


def check_origin(handler):
    """Verifica se a origem da requisição pertence ao mesmo host ou à URL pública do px0.
    Impede ataques CSRF entre portas diferentes de localhost (ex.: preview de PR em porta 8080
    enviando requisições para a porta 7777)."""
    origin = handler.headers.get("Origin")
    if not origin:
        ref = handler.headers.get("Referer")
        if ref:
            origin = f"{urlparse(ref).scheme}://{urlparse(ref).netloc}"
    if not origin:
        return False
    parsed = urlparse(origin)
    origin_netloc = parsed.netloc.lower()
    host = (handler.headers.get("Host") or "").lower()
    public_netloc = urlparse(PX0_PUBLIC_URL).netloc.lower()
    allowed = {host, public_netloc}
    return bool(origin_netloc and origin_netloc in allowed)


# ── Forgejo: listagem de PRs ──────────────────────────────────────────────────


def forgejo_get(path):
    req = urllib.request.Request(f"{FORGEJO_API}{path}", headers={"Authorization": f"token {GITEA_TOKEN}"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.load(r)


def list_prs():
    prs = []
    for full in REPOS:
        try:
            data = forgejo_get(f"/repos/{full}/pulls?state=open&limit=50")
            for p in data:
                prs.append(
                    {
                        "repo": full,
                        "number": p["number"],
                        "title": p["title"],
                        "by": (p.get("user") or {}).get("login", ""),
                    }
                )
        except (urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError) as e:
            prs.append({"repo": full, "error": str(e)})
    return prs


def port_open(port, host="127.0.0.1"):
    try:
        with socket.create_connection((host, port), timeout=0.5):
            return True
    except OSError:
        return False


def open_pr(owner, repo, n):
    key = f"{owner}/{repo}#{n}"
    if current["key"] == key and current["proc"] and current["proc"].poll() is None:
        return  # já está rodando essa sessão
    if current["proc"] and current["proc"].poll() is None:
        current["proc"].terminate()
        try:
            current["proc"].wait(timeout=5)
        except subprocess.TimeoutExpired:
            current["proc"].kill()
    url = f"http://forgejo-tls:8443/{owner}/{repo}/pulls/{n}"
    proc = subprocess.Popen(
        ["/usr/local/bin/px0", "-host", "127.0.0.1", "-port", str(REVIEW_PORT), "-no-open", "-base-path", BASE_PATH, url],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    current["proc"] = proc
    current["key"] = key
    for _ in range(60):
        if port_open(REVIEW_PORT):
            return
        if proc.poll() is not None:
            raise RuntimeError("px0 saiu antes de abrir a porta (PR/repo errado, ou sem permissão no token?)")
        time.sleep(0.5)
    raise RuntimeError("px0 não respondeu a tempo")


# ── Docker (opt-in: exige /var/run/docker.sock montado) ──────────────────────
# Usado pra validar/aplicar configuração sem terminal. Sem o socket montado,
# essas funções só falham de forma clara (container "não encontrado") — a
# tela de PRs e o proxy do px0 continuam funcionando normalmente.


def compose_container_id(service):
    try:
        r = subprocess.run(
            [
                "docker",
                "ps",
                "-q",
                "--filter",
                f"label=com.docker.compose.project={COMPOSE_PROJECT}",
                "--filter",
                f"label=com.docker.compose.service={service}",
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
        ids = r.stdout.strip().splitlines()
        return ids[0] if ids else None
    except (subprocess.SubprocessError, OSError):
        return None


def docker_restart(service):
    cid = compose_container_id(service)
    if not cid:
        return False, f'container do serviço "{service}" não encontrado (docker.sock montado? veja docs/px0.md)'
    try:
        r = subprocess.run(["docker", "restart", cid], capture_output=True, text=True, timeout=30)
        if r.returncode != 0:
            return False, f"falha ao reiniciar {service}: {r.stderr.strip()[:300]}"
        return True, f"{service} reiniciado"
    except (subprocess.SubprocessError, OSError) as e:
        return False, f"falha ao reiniciar {service}: {e}"


# ── Status (API REST que o oreia-seca já expõe) ───────────────────────────────


def fetch_status():
    headers = {"Authorization": f"Bearer {REST_API_KEY}"} if REST_API_KEY else {}
    req = urllib.request.Request(f"{OREIA_SECA_API}/api/status", headers=headers)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.load(r)


def check_candidate(toml_text):
    """Chama POST /api/check do oreia-seca (já rodando) com o candidato de TOML,
    ANTES de gravar nada — ele nunca altera a config em uso, só valida a forma e
    roda as checagens de rede/processo do que mudaria (modelos, executores,
    Forgejo, tokens, Telegram; ver crate::check no Rust). Devolve (True, [resultados])
    quando a forma é válida, ou (False, mensagem/HTTPError) quando o TOML nem
    chega a validar — esse segundo caso é o mesmo "configuração inválida, nada foi
    salvo" de antes, só que validado pelo oreia-seca de verdade, não por um
    arquivo candidato em disco."""
    headers = {"Content-Type": "application/json"}
    if REST_API_KEY:
        headers["Authorization"] = f"Bearer {REST_API_KEY}"
    body = json.dumps({"candidate": toml_text}).encode("utf-8")
    req = urllib.request.Request(f"{OREIA_SECA_API}/api/check", data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return True, json.load(r).get("results", [])
    except urllib.error.HTTPError as e:
        try:
            msg = json.loads(e.read().decode("utf-8", errors="replace")).get("error", str(e))
        except (json.JSONDecodeError, UnicodeDecodeError):
            msg = str(e)
        return False, msg
    except (urllib.error.URLError, TimeoutError) as e:
        return False, f"não consegui falar com {OREIA_SECA_API}/api/check: {e}"


def render_check_results(results):
    """Lista ✅⚠️❌ de uma resposta de check_candidate — avisos e falhas por
    extenso, sem esconder nada (diferente do resumo do Telegram, aqui o espaço
    não é um problema)."""
    if not results:
        return '<p class="text-xs text-muted mt-2 mb-1">(nada mudou desde a config atual)</p>'
    icon = {"ok": "text-ok", "warn": "text-warn", "fail": "text-err"}
    glyph = {"ok": "✅", "warn": "⚠️", "fail": "❌"}
    rows = "".join(
        f'<div class="text-xs py-0.5 {icon.get(r.get("status"), "text-muted")}">'
        f'{glyph.get(r.get("status"), "•")} {html.escape(str(r.get("message", "")))}</div>'
        for r in results
    )
    return f'<div class="mt-2 mb-3 rounded-md border border-border bg-bg/60 p-3">{rows}</div>'


def read_activity(limit=25):
    """Últimas tentativas de fase (`data/escalation.jsonl`) — mais recente
    primeiro. [] se o arquivo não estiver montado ainda (opt-in)."""
    try:
        with open(ESCALATION_LOG_PATH, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except OSError:
        return []
    entries = []
    for line in lines[-limit:]:
        line = line.strip()
        if not line:
            continue
        try:
            entries.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    entries.reverse()
    return entries


def container_stats():
    """`docker stats` dos containers deste projeto — CPU/memória ao vivo —
    MAIS qualquer executor rodando AGORA (opencode/codex em `docker run --rm`,
    efêmeros: só aparecem se uma fase estiver em andamento no instante da
    consulta). [] se o docker.sock não estiver montado (mesmo opt-in da
    Configuração)."""
    try:
        ps = subprocess.run(
            ["docker", "ps", "--format", "{{.ID}}|{{.Image}}"], capture_output=True, text=True, timeout=10
        )
        stats = subprocess.run(
            ["docker", "stats", "--no-stream", "--format", "{{json .}}"], capture_output=True, text=True, timeout=10
        )
    except (subprocess.SubprocessError, OSError):
        return []
    if ps.returncode != 0 or stats.returncode != 0:
        return []
    images = {}
    for line in ps.stdout.splitlines():
        cid, _, image = line.strip().partition("|")
        if cid:
            images[cid] = image
    out = []
    for line in stats.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            raw = json.loads(line)
        except json.JSONDecodeError:
            continue
        name = raw.get("Name", "")
        cid = raw.get("Container") or raw.get("ID", "")
        image = images.get(cid, "")
        cpu = _pct(raw.get("CPUPerc", "0%"))
        mem = _pct(raw.get("MemPerc", "0%"))
        mem_usage = raw.get("MemUsage", "")
        mem_mib = _mem_to_mib(mem_usage)
        if name.startswith(f"{COMPOSE_PROJECT}-"):
            short = re.sub(r"-1$", "", name[len(COMPOSE_PROJECT) + 1 :])
            kind = "service"
        elif image.startswith(f"{COMPOSE_PROJECT}-exec-"):
            short = image[len(COMPOSE_PROJECT) + 1 :].removeprefix("exec-")
            kind = "executor"
        else:
            continue
        out.append(
            {"name": short, "kind": kind, "cpu_pct": cpu, "mem_pct": mem, "mem_usage": mem_usage, "mem_mib": mem_mib}
        )
    out.sort(key=lambda c: (c["kind"] != "executor", c["name"]))
    return out


def _pct(s):
    try:
        return float(str(s).rstrip("%"))
    except ValueError:
        return 0.0


def _mem_to_mib(usage):
    """'63.1MiB / 15.59GiB' -> 63.1 (só a parte usada, em MiB)."""
    m = re.match(r"([\d.]+)\s*(KiB|MiB|GiB|B)", str(usage).split("/")[0].strip())
    if not m:
        return 0.0
    value, unit = float(m.group(1)), m.group(2)
    return {"B": value / 1024 / 1024, "KiB": value / 1024, "MiB": value, "GiB": value * 1024}.get(unit, 0.0)


MEM_HISTORY_MAXLEN = 60  # ~10min a cada 10s
mem_history = collections.deque(maxlen=MEM_HISTORY_MAXLEN)


def sample_memory_loop():
    """Roda em background desde o início do processo, independente de alguém
    estar com a aba de Status aberta — senão o gráfico de picos ficaria vazio
    sempre que você abrisse a página."""
    while True:
        try:
            stats = container_stats()
            mem_history.append({"t": time.time(), "mem": {c["name"]: round(c["mem_mib"], 1) for c in stats}})
        except Exception:  # noqa: BLE001 - nunca pode derrubar a thread de fundo
            pass
        time.sleep(10)


def logs_tail(service="oreia-seca", n=60):
    """Últimas linhas do `docker logs` do container — é o log de verdade da
    aplicação (inclusive conteúdo de LLM/chat quando OREIA_SECA_LLM_DEBUG=1),
    sem precisar instrumentar nada novo no Rust. Cores ANSI removidas."""
    cid = compose_container_id(service)
    if not cid:
        return []
    try:
        r = subprocess.run(["docker", "logs", "--tail", str(n), cid], capture_output=True, text=True, timeout=10)
    except (subprocess.SubprocessError, OSError):
        return []
    text = r.stdout + r.stderr
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    return [line for line in text.splitlines() if line.strip()]


def render_value(v):
    """Qualquer coisa que o /api/status devolver, sem assumir o formato exato
    de cada campo (dict, lista ou escalar) — pra não quebrar se a API mudar."""
    if isinstance(v, dict):
        if not v:
            return "<i>(vazio)</i>"
        rows = "".join(
            f'<tr><td style="padding-right:12px;color:#8b949e;vertical-align:top">{html.escape(str(k))}</td>'
            f"<td>{render_value(val)}</td></tr>"
            for k, val in v.items()
        )
        return f"<table>{rows}</table>"
    if isinstance(v, list):
        if not v:
            return "<i>(vazio)</i>"
        return "<ul>" + "".join(f"<li>{render_value(item)}</li>" for item in v) + "</ul>"
    return html.escape(str(v))


# ── Configuração (opt-in: exige oreia-seca.toml/.env montados rw) ────────────


def read_file_safe(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return f.read()
    except OSError:
        return None


def write_file_atomic(path, content):
    tmp = f"{path}.tmp"
    try:
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(content)
        os.replace(tmp, path)
    except OSError:
        # Bind mount de um arquivo só (ex. docker-compose.override.yml monta
        # exatamente "./oreia-seca.toml:/app-config/oreia-seca.toml"): o
        # diretório pai não deixa CRIAR um arquivo novo pro uid do px0, só o
        # arquivo em si é gravável. Sobrescreve direto, sem o rename atômico.
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)


def parse_env_lines(content):
    """[(key, valor)] pra linha com KEY=valor, ou (None, linha_original) pra
    comentário/linha vazia — preserva pra escrever de volta sem perder nada."""
    out = []
    for line in content.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            out.append((None, line))
            continue
        key, _, value = line.partition("=")
        out.append((key.strip(), value))
    return out


# ── Páginas ───────────────────────────────────────────────────────────────────
# Visual inspirado no https://forgeui.in/ — mas sem adotar os componentes React
# dele: este launcher é um único arquivo Python (biblioteca padrão, sem
# Node/build), e trocar a arquitetura toda por um front-end React só pra ter
# esse visual seria desproporcional. Em vez disso: as cores SÃO as mesmas (os
# tokens OKLCH do tema escuro padrão shadcn/ui — "neutral", extraídos de
# verdade do CSS compilado de forgeui.in, não um palpite) + Tailwind via CDN
# (sem build) + Chart.js (CDN) pro gráfico + um fundo com glow radial/grade
# sutil (o mesmo tipo de `bg-gradient-to-*`/`blur-*` que o bundle deles usa) +
# animações CSS/Tailwind. Mesmo espírito visual, zero dependência de Node.
# Veja docs/px0.md.

HEAD = """
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
<script src="https://cdn.tailwindcss.com"></script>
<script>
  tailwind.config = {
    theme: {
      extend: {
        fontFamily: { sans: ['Inter', 'ui-sans-serif', 'system-ui'] },
        colors: {
          /* Tokens OKLCH reais do tema escuro "neutral" do shadcn/ui, extraídos
             do CSS compilado de forgeui.in — não são um palpite de paleta. */
          bg: 'oklch(14.5% 0 0)', panel: 'oklch(19% 0 0)', border: 'oklch(26.9% 0 0)',
          ink: 'oklch(98.5% 0 0)', muted: 'oklch(70.8% 0 0)',
          accent: 'oklch(58% .19 264)',        /* --chart-1, mais claro pra contraste de botão */
          ok: 'oklch(69.6% .17 162.48)',       /* --chart-2 */
          warn: 'oklch(76.9% .188 70.08)',     /* --chart-3 */
          violet: 'oklch(62.7% .265 303.9)',   /* --chart-4 */
          err: 'oklch(64.5% .246 16.439)',     /* --chart-5 */
        },
        animation: {
          'fade-up': 'fadeUp .5s ease-out both',
          'pulse-dot': 'pulseDot 1.6s ease-in-out infinite',
          'float': 'float 12s ease-in-out infinite',
        },
        keyframes: {
          fadeUp: { '0%': { opacity: 0, transform: 'translateY(8px)' }, '100%': { opacity: 1, transform: 'translateY(0)' } },
          pulseDot: { '0%,100%': { opacity: 1 }, '50%': { opacity: .35 } },
          float: { '0%,100%': { transform: 'translate(0,0)' }, '50%': { transform: 'translate(24px,-24px)' } },
        },
      },
    },
  };
</script>
<style>
  body{background:oklch(14.5% 0 0);font-family:'Inter',ui-sans-serif,system-ui}
  ::-webkit-scrollbar{width:10px;height:10px}
  ::-webkit-scrollbar-thumb{background:oklch(26.9% 0 0);border-radius:6px}
  .bar-fill{transition:width .8s cubic-bezier(.2,.8,.2,1)}
  textarea.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
  .bg-grid{background-image:linear-gradient(oklch(26.9% 0 0 / 40%) 1px,transparent 1px),linear-gradient(90deg,oklch(26.9% 0 0 / 40%) 1px,transparent 1px);background-size:40px 40px}
  .glow{position:fixed;border-radius:9999px;filter:blur(100px);pointer-events:none}
  .tabular{font-variant-numeric:tabular-nums}
</style>
"""

# Fundo: grade sutil + dois blobs de glow radial flutuando — o mesmo tipo de
# `bg-gradient-to-*`/`blur-3xl` encontrado no CSS compilado de forgeui.in,
# recriado em CSS puro (sem o React deles).
BACKDROP = """
<div class="fixed inset-0 bg-grid opacity-[0.15] pointer-events-none z-0"></div>
<div class="glow w-[480px] h-[480px] -top-48 -left-48 bg-accent/25 animate-float"></div>
<div class="glow w-[420px] h-[420px] top-1/4 -right-24 bg-violet/15 animate-float" style="animation-delay:-6s"></div>
"""


def banner(kind, message):
    cls = "border-ok/40 bg-ok/10 text-ok" if kind == "ok" else "border-err/40 bg-err/10 text-err"
    return f'<div class="animate-fade-up rounded-md border {cls} px-3 py-2 text-sm mb-3">{message}</div>'


def state_badge_class(state):
    return {
        "ok": "bg-ok/15 text-ok",
        "unreachable": "bg-err/15 text-err",
        "not_configured": "bg-muted/15 text-muted",
    }.get(str(state), "bg-muted/15 text-muted")


def error_page(title, message):
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(title)}</title>{HEAD}</head>
<body class="bg-bg text-ink min-h-screen flex items-center justify-center relative overflow-hidden">
{BACKDROP}
  <div class="relative z-10 max-w-md mx-auto px-6 text-center animate-fade-up">
    <div class="text-4xl mb-3">⚠</div>
    <p class="text-sm text-muted">{message}</p>
    <a href="/" class="inline-block mt-5 text-sm text-accent hover:underline">Voltar</a>
  </div>
</body></html>"""


def login_page(error_html=""):
    oauth_html = ""
    if OAUTH_CLIENT_ID:
        oauth_html = """
          <a href="/oauth/login" class="block w-full text-center rounded-lg bg-gradient-to-br from-accent to-violet text-white font-medium py-2.5 transition-all hover:shadow-lg hover:shadow-accent/30 hover:brightness-110">
            Entrar com o Forgejo
          </a>
          <div class="flex items-center gap-3 my-5 text-xs text-muted">
            <div class="flex-1 h-px bg-border"></div>ou com usuário/senha<div class="flex-1 h-px bg-border"></div>
          </div>"""
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Entrar — Revisar PRs</title>{HEAD}</head>
<body class="bg-bg text-ink min-h-screen flex items-center justify-center relative overflow-hidden">
{BACKDROP}
  <div class="relative z-10 w-full max-w-sm mx-auto px-6 animate-fade-up">
    <div class="text-center mb-8">
      <div class="text-2xl font-semibold tracking-tight bg-gradient-to-r from-ink to-muted bg-clip-text text-transparent">OREIA · px0</div>
      <p class="text-muted text-sm mt-1">Revisão de PR, status e configuração</p>
    </div>
    <div class="bg-panel/80 backdrop-blur border border-border rounded-2xl p-6 shadow-2xl shadow-black/60">
      {oauth_html}
      <form method="post" action="/login" class="space-y-4">
        <div>
          <label class="block text-xs text-muted mb-1">Usuário</label>
          <input name="user" autofocus required class="w-full rounded-md bg-bg border border-border px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent/50">
        </div>
        <div>
          <label class="block text-xs text-muted mb-1">Senha ou token pessoal</label>
          <input name="pass" type="password" required class="w-full rounded-md bg-bg border border-border px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent/50">
        </div>
        {error_html}
        <button type="submit" class="w-full rounded-lg border border-border hover:border-accent/60 hover:bg-accent/10 text-sm font-medium py-2.5 transition-all">Entrar</button>
      </form>
    </div>
  </div>
</body></html>"""


def page_shell(title, user, active, body_html):
    user_html = ""
    tabs = ""
    if user:
        links = [("/", "PRs"), ("/status", "Status"), ("/config", "Configuração")]
        underline = '<span class="absolute left-0 -bottom-0.5 h-0.5 w-full bg-gradient-to-r from-accent to-violet rounded"></span>'
        tab_parts = []
        for href, label in links:
            is_active = href == active
            cls = "text-ink font-medium" if is_active else "text-muted hover:text-ink"
            tab_parts.append(f'<a href="{href}" class="relative pb-1 text-sm transition {cls}">{label}{underline if is_active else ""}</a>')
        tabs = "".join(tab_parts)
        user_html = (
            f'<div class="flex items-center gap-3 text-sm text-muted">'
            f'<span class="flex items-center gap-1.5"><span class="h-2 w-2 rounded-full bg-ok animate-pulse-dot"></span>{html.escape(user)}</span>'
            f'<a href="/logout" class="hover:text-ink transition">sair</a></div>'
        )
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(title)}</title>{HEAD}</head>
<body class="bg-bg text-ink min-h-screen relative overflow-x-hidden">
{BACKDROP}
<nav class="relative z-10 max-w-5xl mx-auto px-6 py-5 flex items-center justify-between border-b border-border/60">
  <div class="flex items-center gap-7">
    <span class="font-semibold tracking-tight bg-gradient-to-r from-ink to-muted bg-clip-text text-transparent">OREIA · px0</span>
    <div class="flex items-center gap-5">{tabs}</div>
  </div>
  {user_html}
</nav>
<main class="relative z-10 max-w-5xl mx-auto px-6 pb-16">
{body_html}
</main>
</body></html>"""


def main_page(user, rows_html):
    body = f"""
      <div class="flex items-center justify-between mt-6 mb-6">
        <h1 class="text-xl font-semibold tracking-tight">PRs abertos</h1>
        <span class="text-xs text-muted">clique em Revisar pra abrir no px0</span>
      </div>
      <div class="space-y-3">{rows_html}</div>"""
    return page_shell("Revisar PRs", user, "/", body)


def stat_tile(key, label, value, color_class, bar_pct):
    return f"""
      <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up overflow-hidden relative">
        <div class="text-xs text-muted">{label}</div>
        <div id="tile-{key}-val" class="tabular text-2xl font-semibold mt-1 text-{color_class}">{value}</div>
        <div class="h-1.5 bg-bg rounded-full mt-3 overflow-hidden">
          <div id="tile-{key}-bar" class="bar-fill h-full rounded-full bg-{color_class}" style="width:{bar_pct:.0f}%"></div>
        </div>
      </div>"""


def fmt_ms(ms):
    try:
        ms = float(ms)
    except (TypeError, ValueError):
        return "?"
    return f"{ms / 1000:.1f}s" if ms >= 1000 else f"{ms:.0f}ms"


def fmt_ago(iso_ts):
    # `at` vem em UTC (sufixo "Z") — calendar.timegm (não time.mktime, que
    # assumiria fuso local) pra não depender do TZ do container.
    try:
        t = calendar.timegm(time.strptime(iso_ts[:19], "%Y-%m-%dT%H:%M:%S"))
        delta = max(time.time() - t, 0)
    except (ValueError, TypeError):
        return "?"
    if delta < 60:
        return f"há {int(delta)}s"
    if delta < 3600:
        return f"há {int(delta / 60)}min"
    return f"há {int(delta / 3600)}h"


def activity_row(e):
    passed = e.get("passed")
    icon = "bg-ok" if passed else "bg-err"
    reason = f'<div class="text-xs text-err mt-0.5 truncate">{html.escape(str(e.get("reason") or ""))}</div>' if not passed and e.get("reason") else ""
    return f"""
      <div class="flex items-start gap-3 py-2.5 border-b border-border last:border-0">
        <span class="h-2 w-2 rounded-full {icon} mt-1.5 shrink-0"></span>
        <div class="min-w-0 flex-1">
          <div class="flex items-center justify-between gap-2">
            <span class="text-sm font-medium truncate">{html.escape(str(e.get("role", "")))} <span class="text-muted">· fase {e.get("phase", "?")} · degrau {e.get("step", "?")}</span></span>
            <span class="text-xs text-muted shrink-0 tabular">{fmt_ago(e.get("at", ""))}</span>
          </div>
          <div class="flex items-center gap-2 mt-1">
            <span class="text-xs px-2 py-0.5 rounded-full bg-accent/15 text-accent">{html.escape(str(e.get("executor", "")))}</span>
            <span class="text-xs text-muted">{html.escape(str(e.get("model", "")))}</span>
            <span class="text-xs text-muted ml-auto tabular">{fmt_ms(e.get("duration_ms"))}</span>
          </div>
          {reason}
        </div>
      </div>"""


def container_row(c):
    badge = (
        '<span class="text-xs px-1.5 py-0.5 rounded bg-violet/15 text-violet ml-1.5">executor</span>'
        if c.get("kind") == "executor"
        else ""
    )
    return f"""
      <div class="py-2.5 border-b border-border last:border-0">
        <div class="flex items-center justify-between text-sm mb-1.5">
          <span class="font-medium">{html.escape(c["name"])}{badge}</span>
          <span class="text-xs text-muted tabular">{html.escape(c["mem_usage"])}</span>
        </div>
        <div class="flex items-center gap-2">
          <span class="text-xs text-muted w-8">CPU</span>
          <div class="h-1.5 flex-1 bg-bg rounded-full overflow-hidden"><div class="bar-fill h-full rounded-full bg-accent" style="width:{min(c["cpu_pct"], 100):.0f}%"></div></div>
          <span class="text-xs text-muted w-10 text-right tabular">{c["cpu_pct"]:.1f}%</span>
        </div>
        <div class="flex items-center gap-2 mt-1">
          <span class="text-xs text-muted w-8">Mem</span>
          <div class="h-1.5 flex-1 bg-bg rounded-full overflow-hidden"><div class="bar-fill h-full rounded-full bg-violet" style="width:{min(c["mem_pct"], 100):.0f}%"></div></div>
          <span class="text-xs text-muted w-10 text-right tabular">{c["mem_pct"]:.1f}%</span>
        </div>
      </div>"""


def status_page(user):
    try:
        data = fetch_status()
    except (urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError, TimeoutError) as e:
        body = f'<h1 class="text-xl font-semibold mt-6 mb-4">Status</h1>{banner("err", f"Falha ao consultar o oreia-seca: {html.escape(str(e))}")}'
        return page_shell("Status", user, "/status", body)
    activity = read_activity()
    containers = container_stats()
    logs = logs_tail()
    history = list(mem_history)

    q = data.get("queue", {}) or {}
    running, queued = int(q.get("running", 0)), int(q.get("queued", 0))
    completed, failed = int(q.get("completed", 0)), int(q.get("failed", 0))
    total = max(running + queued + completed + failed, 1)
    tiles = (
        stat_tile("running", "Rodando", running, "accent", running / total * 100)
        + stat_tile("queued", "Na fila", queued, "warn", queued / total * 100)
        + stat_tile("completed", "Concluídas", completed, "ok", completed / total * 100)
        + stat_tile("failed", "Falhas", failed, "err", failed / total * 100)
    )

    models = data.get("models", []) or []
    model_rows = "".join(
        f'<div class="flex items-center justify-between py-2 border-b border-border last:border-0">'
        f'<div><span class="font-medium text-sm">{html.escape(str(m.get("key", "")))}</span>'
        f'<span class="text-xs text-muted ml-2">{html.escape(str(m.get("model", "")))}</span></div>'
        f'<span class="text-xs px-2 py-0.5 rounded-full {state_badge_class(m.get("state"))}">{html.escape(str(m.get("state", "")))}</span></div>'
        for m in models
    ) or '<p class="text-xs text-muted">nenhum</p>'

    roles = data.get("roles", []) or []
    role_rows = "".join(
        f'<div class="flex items-center justify-between py-2 border-b border-border last:border-0">'
        f'<span class="text-sm">{html.escape(str(r.get("role", "")))}</span>'
        f'<span class="text-xs text-muted">→ {html.escape(str(r.get("model", "")))}</span>'
        f'<span class="h-2 w-2 rounded-full {"bg-ok" if r.get("ready") else "bg-err"}"></span></div>'
        for r in roles
    ) or '<p class="text-xs text-muted">nenhum</p>'

    agents = data.get("agents", []) or []
    agent_pills = "".join(
        f'<span class="inline-flex items-center gap-1.5 text-xs bg-bg border border-border rounded-full px-3 py-1">'
        f'<span class="h-1.5 w-1.5 rounded-full bg-ok"></span>{html.escape(str(a))}</span>'
        for a in agents
    ) or '<span class="text-xs text-muted">nenhum</span>'

    pending = data.get("pending_approvals", []) or []
    pending_html = "".join(f'<li class="py-1 text-sm">{render_value(p)}</li>' for p in pending) or '<p class="text-xs text-muted">nenhuma</p>'
    active = data.get("active_work", []) or []
    active_html = "".join(f'<li class="py-1 text-sm font-mono">{render_value(a)}</li>' for a in active) or '<p class="text-xs text-muted">nenhum</p>'

    activity_html = "".join(activity_row(e) for e in activity) or '<p class="text-xs text-muted py-2">Nenhuma atividade ainda (ou o log não está montado — veja docs/px0.md).</p>'
    containers_html = "".join(container_row(c) for c in containers) or '<p class="text-xs text-muted py-2">docker.sock não montado — veja docs/px0.md.</p>'
    logs_html = "\n".join(html.escape(line) for line in logs) or "(sem logs — docker.sock não montado, ou nada ainda)"

    initial_json = json.dumps(
        {"status": data, "activity": activity, "containers": containers, "logs": logs, "mem_history": history}
    ).replace("</", "<\\/")

    body = f"""
      <div class="flex items-center justify-between mt-6 mb-6">
        <h1 class="text-xl font-semibold tracking-tight">Status</h1>
        <span class="flex items-center gap-1.5 text-xs text-muted"><span id="liveDot" class="h-2 w-2 rounded-full bg-ok animate-pulse-dot"></span>ao vivo</span>
      </div>
      <div id="tiles" class="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">{tiles}</div>

      <div class="grid md:grid-cols-2 gap-4 mb-4">
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-1">Containers <span class="text-xs text-muted font-normal">— serviços e executores rodando agora</span></div>
          <div id="containersList">{containers_html}</div>
        </div>
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-3">Memória ao longo do tempo <span class="text-xs text-muted font-normal">— últimos ~10min</span></div>
          <canvas id="memChart" height="150"></canvas>
        </div>
      </div>

      <div class="bg-panel border border-border rounded-xl p-4 mb-4 animate-fade-up">
        <div class="text-sm font-medium mb-1">Atividade recente <span class="text-xs text-muted font-normal">— papel, fase, executor e tempo de cada tentativa</span></div>
        <div id="activityList" class="max-h-96 overflow-y-auto">{activity_html}</div>
      </div>

      <div class="grid md:grid-cols-2 gap-4 mb-4">
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-3">Duração das últimas tentativas</div>
          <canvas id="durationChart" height="150"></canvas>
        </div>
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-1">Logs <span class="text-xs text-muted font-normal">— últimas linhas de docker logs oreia-seca</span></div>
          <pre id="logsBox" class="mono text-[11px] leading-snug text-muted max-h-72 overflow-y-auto whitespace-pre-wrap break-all">{logs_html}</pre>
        </div>
      </div>

      <div class="grid md:grid-cols-3 gap-4 mb-4">
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-2">Papéis → modelo</div>
          <div id="rolesList">{role_rows}</div>
        </div>
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-2">Modelos</div>
          <div id="modelsList">{model_rows}</div>
        </div>
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-3">Agentes</div>
          <div id="agentsList" class="flex flex-wrap gap-2">{agent_pills}</div>
        </div>
      </div>
      <div class="grid md:grid-cols-2 gap-4 mb-6">
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-2">Aprovações pendentes</div><ul id="pendingList">{pending_html}</ul>
        </div>
        <div class="bg-panel border border-border rounded-xl p-4 animate-fade-up">
          <div class="text-sm font-medium mb-2">Trabalho ativo</div><ul id="activeList">{active_html}</ul>
        </div>
      </div>
      <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.4/dist/chart.umd.min.js"></script>
      <script>
        // Atualiza em memória a cada 5s (fetch + patch do DOM) — sem recarregar a
        // página (o jeito antigo, <meta refresh>, piscava e reiniciava as animações).
        function esc(s) {{
          return String(s).replace(/[&<>"']/g, c => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[c]));
        }}
        function renderValue(v) {{
          if (v && typeof v === 'object' && !Array.isArray(v)) {{
            const keys = Object.keys(v);
            if (!keys.length) return '<i>(vazio)</i>';
            return '<table>' + keys.map(k => `<tr><td style="padding-right:12px;color:#8b949e;vertical-align:top">${{esc(k)}}</td><td>${{renderValue(v[k])}}</td></tr>`).join('') + '</table>';
          }}
          if (Array.isArray(v)) {{
            if (!v.length) return '<i>(vazio)</i>';
            return '<ul>' + v.map(item => `<li>${{renderValue(item)}}</li>`).join('') + '</ul>';
          }}
          return esc(v);
        }}
        function fmtMs(ms) {{
          ms = +ms || 0;
          return ms >= 1000 ? (ms / 1000).toFixed(1) + 's' : Math.round(ms) + 'ms';
        }}
        function fmtAgo(iso) {{
          const delta = Math.max((Date.now() - new Date(iso).getTime()) / 1000, 0);
          if (delta < 60) return 'há ' + Math.round(delta) + 's';
          if (delta < 3600) return 'há ' + Math.round(delta / 60) + 'min';
          return 'há ' + Math.round(delta / 3600) + 'h';
        }}
        function bumpTile(key, value, total) {{
          const valEl = document.getElementById('tile-' + key + '-val');
          const barEl = document.getElementById('tile-' + key + '-bar');
          if (!valEl) return;
          if (valEl.textContent !== String(value)) {{
            valEl.animate([{{ transform: 'scale(1.3)' }}, {{ transform: 'scale(1)' }}], {{ duration: 350, easing: 'ease-out' }});
          }}
          valEl.textContent = value;
          barEl.style.width = (total > 0 ? (value / total * 100) : 0) + '%';
        }}
        function renderStatus(payload) {{
          const data = payload.status || {{}};
          const q = data.queue || {{}};
          const vals = {{ running: +q.running || 0, queued: +q.queued || 0, completed: +q.completed || 0, failed: +q.failed || 0 }};
          const total = Math.max(vals.running + vals.queued + vals.completed + vals.failed, 1);
          bumpTile('running', vals.running, total);
          bumpTile('queued', vals.queued, total);
          bumpTile('completed', vals.completed, total);
          bumpTile('failed', vals.failed, total);

          const roles = data.roles || [];
          document.getElementById('rolesList').innerHTML = roles.map(r => `
            <div class="flex items-center justify-between py-2 border-b border-border last:border-0">
              <span class="text-sm">${{esc(r.role || '')}}</span>
              <span class="text-xs text-muted">→ ${{esc(r.model || '')}}</span>
              <span class="h-2 w-2 rounded-full ${{r.ready ? 'bg-ok' : 'bg-err'}}"></span>
            </div>`).join('') || '<p class="text-xs text-muted">nenhum</p>';

          const models = data.models || [];
          const stateBadge = s => ({{ ok: 'bg-ok/15 text-ok', unreachable: 'bg-err/15 text-err', not_configured: 'bg-muted/15 text-muted' }}[s] || 'bg-muted/15 text-muted');
          document.getElementById('modelsList').innerHTML = models.map(m => `
            <div class="flex items-center justify-between py-2 border-b border-border last:border-0">
              <div><span class="font-medium text-sm">${{esc(m.key || '')}}</span><span class="text-xs text-muted ml-2">${{esc(m.model || '')}}</span></div>
              <span class="text-xs px-2 py-0.5 rounded-full ${{stateBadge(m.state)}}">${{esc(m.state || '')}}</span>
            </div>`).join('') || '<p class="text-xs text-muted">nenhum</p>';

          const agents = data.agents || [];
          document.getElementById('agentsList').innerHTML = agents.map(a => `
            <span class="inline-flex items-center gap-1.5 text-xs bg-bg border border-border rounded-full px-3 py-1">
              <span class="h-1.5 w-1.5 rounded-full bg-ok"></span>${{esc(a)}}</span>`).join('') || '<span class="text-xs text-muted">nenhum</span>';

          const pending = data.pending_approvals || [];
          document.getElementById('pendingList').innerHTML = pending.map(p => `<li class="py-1 text-sm">${{renderValue(p)}}</li>`).join('') || '<p class="text-xs text-muted">nenhuma</p>';
          const active = data.active_work || [];
          document.getElementById('activeList').innerHTML = active.map(a => `<li class="py-1 text-sm font-mono">${{renderValue(a)}}</li>`).join('') || '<p class="text-xs text-muted">nenhum</p>';

          const activity = payload.activity || [];
          document.getElementById('activityList').innerHTML = activity.map(e => `
            <div class="flex items-start gap-3 py-2.5 border-b border-border last:border-0">
              <span class="h-2 w-2 rounded-full ${{e.passed ? 'bg-ok' : 'bg-err'}} mt-1.5 shrink-0"></span>
              <div class="min-w-0 flex-1">
                <div class="flex items-center justify-between gap-2">
                  <span class="text-sm font-medium truncate">${{esc(e.role || '')}} <span class="text-muted">· fase ${{esc(e.phase ?? '?')}} · degrau ${{esc(e.step ?? '?')}}</span></span>
                  <span class="text-xs text-muted shrink-0 tabular">${{fmtAgo(e.at)}}</span>
                </div>
                <div class="flex items-center gap-2 mt-1">
                  <span class="text-xs px-2 py-0.5 rounded-full bg-accent/15 text-accent">${{esc(e.executor || '')}}</span>
                  <span class="text-xs text-muted">${{esc(e.model || '')}}</span>
                  <span class="text-xs text-muted ml-auto tabular">${{fmtMs(e.duration_ms)}}</span>
                </div>
                ${{(!e.passed && e.reason) ? `<div class="text-xs text-err mt-0.5 truncate">${{esc(e.reason)}}</div>` : ''}}
              </div>
            </div>`).join('') || '<p class="text-xs text-muted py-2">Nenhuma atividade ainda.</p>';

          const containers = payload.containers || [];
          document.getElementById('containersList').innerHTML = containers.map(c => `
            <div class="py-2.5 border-b border-border last:border-0">
              <div class="flex items-center justify-between text-sm mb-1.5">
                <span class="font-medium">${{esc(c.name)}}${{c.kind === 'executor' ? '<span class="text-xs px-1.5 py-0.5 rounded bg-violet/15 text-violet ml-1.5">executor</span>' : ''}}</span>
                <span class="text-xs text-muted tabular">${{esc(c.mem_usage)}}</span>
              </div>
              <div class="flex items-center gap-2">
                <span class="text-xs text-muted w-8">CPU</span>
                <div class="h-1.5 flex-1 bg-bg rounded-full overflow-hidden"><div class="bar-fill h-full rounded-full bg-accent" style="width:${{Math.min(c.cpu_pct,100)}}%"></div></div>
                <span class="text-xs text-muted w-10 text-right tabular">${{c.cpu_pct.toFixed(1)}}%</span>
              </div>
              <div class="flex items-center gap-2 mt-1">
                <span class="text-xs text-muted w-8">Mem</span>
                <div class="h-1.5 flex-1 bg-bg rounded-full overflow-hidden"><div class="bar-fill h-full rounded-full bg-violet" style="width:${{Math.min(c.mem_pct,100)}}%"></div></div>
                <span class="text-xs text-muted w-10 text-right tabular">${{c.mem_pct.toFixed(1)}}%</span>
              </div>
            </div>`).join('') || '<p class="text-xs text-muted py-2">docker.sock não montado.</p>';

          const logs = payload.logs || [];
          const logsBox = document.getElementById('logsBox');
          if (logsBox) {{
            const atBottom = logsBox.scrollTop + logsBox.clientHeight >= logsBox.scrollHeight - 20;
            logsBox.textContent = logs.join('\\n') || '(sem logs)';
            if (atBottom) logsBox.scrollTop = logsBox.scrollHeight;
          }}

          const chartEntries = activity.slice(0, 12).reverse();
          if (durationChart) {{
            durationChart.data.labels = chartEntries.map(e => (e.role || '').replace('dev-', '') + '·' + (e.phase ?? ''));
            durationChart.data.datasets[0].data = chartEntries.map(e => e.duration_ms || 0);
            durationChart.data.datasets[0].backgroundColor = chartEntries.map(e => e.passed ? '#5ecf8f' : '#f36a6a');
            durationChart.update();
          }}

          const history = payload.mem_history || [];
          if (memChart && history.length) {{
            const names = [...new Set(history.flatMap(h => Object.keys(h.mem || {{}})))];
            const palette = ['#4b93f7', '#5ecf8f', '#f3c969', '#b084f0', '#f36a6a', '#8b949e'];
            memChart.data.labels = history.map(h => new Date(h.t * 1000).toLocaleTimeString().slice(0, 5));
            memChart.data.datasets = names.map((name, i) => ({{
              label: name,
              data: history.map(h => (h.mem || {{}})[name] ?? null),
              borderColor: palette[i % palette.length],
              backgroundColor: palette[i % palette.length],
              tension: 0.3,
              pointRadius: 0,
              spanGaps: true,
            }}));
            memChart.update();
          }}
        }}
        let durationChart, memChart;
        (function() {{
          const initial = {initial_json};
          durationChart = new Chart(document.getElementById('durationChart'), {{
            type: 'bar',
            data: {{ labels: [], datasets: [{{ data: [], backgroundColor: [], borderRadius: 4, maxBarThickness: 28 }}] }},
            options: {{
              plugins: {{ legend: {{ display: false }} }},
              scales: {{
                x: {{ ticks: {{ color: '#8b949e', font: {{ size: 10 }} }}, grid: {{ display: false }} }},
                y: {{ ticks: {{ color: '#8b949e', callback: v => fmtMs(v) }}, grid: {{ color: '#1f2733' }} }},
              }},
            }},
          }});
          memChart = new Chart(document.getElementById('memChart'), {{
            type: 'line',
            data: {{ labels: [], datasets: [] }},
            options: {{
              plugins: {{ legend: {{ position: 'bottom', labels: {{ color: '#8b949e', boxWidth: 10, font: {{ size: 10 }} }} }} }},
              scales: {{
                x: {{ ticks: {{ color: '#8b949e', font: {{ size: 9 }}, maxTicksLimit: 6 }}, grid: {{ display: false }} }},
                y: {{ ticks: {{ color: '#8b949e', callback: v => v + ' MiB' }}, grid: {{ color: '#1f2733' }} }},
              }},
            }},
          }});
          renderStatus(initial);
          setInterval(() => {{
            fetch('/status.json').then(r => r.json()).then(renderStatus).catch(() => {{
              const dot = document.getElementById('liveDot');
              if (dot) dot.classList.replace('bg-ok', 'bg-err');
            }});
          }}, 5000);
        }})();
      </script>"""
    return page_shell("Status", user, "/status", body)


def config_page(user, toml_ok="", toml_error="", env_ok="", env_error="", csrf_token=""):
    toml_content = read_file_safe(TOML_PATH)
    env_content = read_file_safe(ENV_PATH)
    if toml_content is None and env_content is None:
        body = (
            '<h1 class="text-xl font-semibold mt-6 mb-4">Configuração</h1>'
            + banner(
                "err",
                "Não disponível — falta montar <code>oreia-seca.toml</code>/<code>.env</code> e o "
                "<code>docker.sock</code> no container do px0. Veja "
                "<code>docker-compose.override.example.yml</code> e docs/px0.md.",
            )
        )
        return page_shell("Configuração", user, "/config", body)

    sections = ['<h1 class="text-xl font-semibold mt-6 mb-6">Configuração</h1>']

    if toml_content is not None:
        sections.append(f"""
          <div class="bg-panel border border-border rounded-xl p-5 mb-6 animate-fade-up">
            <div class="flex items-center justify-between mb-3">
              <h2 class="text-sm font-medium">oreia-seca.toml</h2>
              <span class="text-xs text-muted">validado antes de salvar</span>
            </div>
            {toml_ok}{toml_error}
            <form method="post" action="/config/toml">
              <input type="hidden" name="csrf" value="{html.escape(csrf_token)}">
              <textarea name="toml" rows="20" class="mono w-full text-xs bg-bg border border-border rounded-lg p-3 focus:outline-none focus:ring-2 focus:ring-accent/50">{html.escape(toml_content)}</textarea>
              <div class="mt-3 flex items-center gap-3">
                <button type="submit" class="rounded-lg bg-accent hover:bg-blue-500 text-white text-sm font-medium px-4 py-2 transition-all hover:shadow-lg hover:shadow-accent/30">Validar e salvar</button>
                <span class="text-xs text-muted">reinicia o oreia-seca</span>
              </div>
            </form>
          </div>""")

    if env_content is not None:
        rows = []
        for key, value in parse_env_lines(env_content):
            if key is None:
                continue
            if SECRET_KEY_RE.search(key):
                rows.append(
                    f'<div><label class="block text-xs text-muted mb-1">{html.escape(key)}</label>'
                    f'<input name="env__{html.escape(key)}" type="password" placeholder="(definido — deixe em branco para manter)" '
                    f'class="w-full rounded-md bg-bg border border-border px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-accent/50"></div>'
                )
            else:
                rows.append(
                    f'<div><label class="block text-xs text-muted mb-1">{html.escape(key)}</label>'
                    f'<input name="env__{html.escape(key)}" value="{html.escape(value)}" '
                    f'class="w-full rounded-md bg-bg border border-border px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-accent/50"></div>'
                )
        sections.append(f"""
          <div class="bg-panel border border-border rounded-xl p-5 animate-fade-up">
            <div class="flex items-center justify-between mb-1">
              <h2 class="text-sm font-medium">.env</h2>
              <span class="text-xs text-muted">reinicia oreia-seca e px0</span>
            </div>
            <p class="text-xs text-muted mb-3">Campos de segredo aparecem em branco — preencha só pra trocar.</p>
            {env_ok}{env_error}
            <form method="post" action="/config/env">
              <input type="hidden" name="csrf" value="{html.escape(csrf_token)}">
              <div class="grid md:grid-cols-2 gap-3">{"".join(rows)}</div>
              <button type="submit" class="mt-4 rounded-lg bg-accent hover:bg-blue-500 text-white text-sm font-medium px-4 py-2 transition-all hover:shadow-lg hover:shadow-accent/30">Salvar</button>
            </form>
          </div>""")

    return page_shell("Configuração", user, "/config", "\n".join(sections))


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _html(self, code, body, extra_headers=None):
        b = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        for k, v in (extra_headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(b)

    def _json(self, code, obj):
        b = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _redirect(self, location, extra_headers=None):
        # Content-Length: 0 explícito — sem isso, em HTTP/1.1 com keep-alive
        # (o padrão aqui), o cliente fica esperando um corpo que nunca chega.
        self.send_response(302)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        for k, v in (extra_headers or {}).items():
            self.send_header(k, v)
        self.end_headers()

    def _body(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        return self.rfile.read(length) if length else b""

    # ── proxy reverso pro px0 da sessão atual, sob BASE_PATH ─────────────────
    def _proxy(self):
        if not (current["proc"] and current["proc"].poll() is None):
            self._html(502, error_page("Sem sessão", "Nenhuma sessão de PR ativa — volte e clique em Revisar."))
            return
        body = self._body()
        fwd_headers = {k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection")}
        try:
            conn = http.client.HTTPConnection("127.0.0.1", REVIEW_PORT, timeout=30)
            conn.request(self.command, self.path, body=body or None, headers=fwd_headers)
            resp = conn.getresponse()
        except OSError:
            self._html(502, error_page("Sessão caiu", "Sessão do px0 caiu — volte e clique em Revisar de novo."))
            return
        self.send_response(resp.status)
        for k, v in resp.getheaders():
            if k.lower() in ("connection", "transfer-encoding"):
                continue
            self.send_header(k, v)
        self.close_connection = True
        self.end_headers()
        while True:
            chunk = resp.read(8192)
            if not chunk:
                break
            try:
                self.wfile.write(chunk)
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                break
        conn.close()

    # ── roteamento ────────────────────────────────────────────────────────────
    def do_GET(self):
        u = urlparse(self.path)

        if u.path == "/login":
            self._html(200, login_page())
            return

        if u.path == "/oauth/login":
            if not OAUTH_CLIENT_ID:
                self._html(400, error_page("OAuth2", "OAuth2 não configurado — rode o bootstrap. Veja docs/px0.md."))
                return
            state = secrets.token_urlsafe(24)
            oauth_states[state] = time.time() + OAUTH_STATE_TTL
            self._redirect(oauth_authorize_url(state))
            return

        if u.path == "/oauth/callback":
            q = parse_qs(u.query)
            state = q.get("state", [""])[0]
            code = q.get("code", [""])[0]
            expires = oauth_states.pop(state, None)
            if not expires or expires < time.time() or not code:
                self._html(400, error_page("Login expirado", "Login expirado ou inválido — volte e tente de novo."))
                return
            try:
                login = oauth_exchange_code(code)
            except (urllib.error.HTTPError, urllib.error.URLError, KeyError, TimeoutError) as e:
                self._html(502, error_page("Falha no login", f"Falha ao validar login com o Forgejo: {html.escape(str(e))}"))
                return
            if not login or not is_allowed(login):
                self._html(403, login_page(banner("err", f'Conta "{html.escape(login or "?")}" não autorizada.')))
                return
            sid = new_session(login)
            self._redirect("/", {"Set-Cookie": f"{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Lax"})
            return

        if u.path == "/logout":
            s = session_from_cookie(self)
            jar = cookies.SimpleCookie()
            jar.load(self.headers.get("Cookie", ""))
            if SESSION_COOKIE in jar:
                sessions.pop(jar[SESSION_COOKIE].value, None)
            self._redirect("/login", {"Set-Cookie": f"{SESSION_COOKIE}=; Path=/; Max-Age=0"})
            return

        session = session_from_cookie(self)
        if not session:
            self._redirect("/login")
            return

        if u.path == "/":
            rows = []
            for i, pr in enumerate(list_prs()):
                if "error" in pr:
                    rows.append(banner("err", f'{html.escape(pr["repo"])}: {html.escape(pr["error"])}'))
                    continue
                link = f"/open?repo={quote(pr['repo'])}&n={pr['number']}"
                rows.append(f"""
                  <div class="animate-fade-up bg-panel border border-border rounded-xl p-4 flex items-center justify-between gap-4 hover:border-accent/40 transition-colors" style="animation-delay:{i * 40}ms">
                    <div class="min-w-0">
                      <span class="font-mono text-xs text-muted bg-bg border border-border rounded px-1.5 py-0.5">{html.escape(pr["repo"])}#{pr["number"]}</span>
                      <div class="mt-1.5 font-medium truncate">{html.escape(pr["title"])}</div>
                      <div class="text-xs text-muted mt-0.5">por {html.escape(pr["by"])}</div>
                    </div>
                    <a href="{link}" class="shrink-0 rounded-lg bg-accent hover:bg-blue-500 text-white text-sm font-medium px-4 py-2 transition-all hover:shadow-lg hover:shadow-accent/30">Revisar</a>
                  </div>""")
            if not rows:
                rows = ['<div class="text-center py-16 text-muted animate-fade-up"><div class="text-3xl mb-2">🎉</div>Nenhum PR aberto.</div>']
            self._html(200, main_page(html.escape(session["user"]), "\n".join(rows)))
            return

        if u.path == "/status":
            self._html(200, status_page(session["user"]))
            return

        if u.path == "/status.json":
            # Pro polling do /status em primeiro plano, sem recarregar a página
            # inteira (que piscava e reiniciava as animações de entrada). Um
            # payload só (fila/papéis/modelos + atividade + containers + logs +
            # histórico de memória) em vez de várias requisições separadas.
            try:
                status = fetch_status()
            except (urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError, TimeoutError) as e:
                self._json(502, {"error": str(e)})
                return
            self._json(
                200,
                {
                    "status": status,
                    "activity": read_activity(),
                    "containers": container_stats(),
                    "logs": logs_tail(),
                    "mem_history": list(mem_history),
                },
            )
            return

        if u.path == "/config":
            self._html(200, config_page(session["user"], csrf_token=session.get("csrf", "")))
            return

        if u.path == "/open":
            q = parse_qs(u.query)
            full = q.get("repo", [""])[0]
            n = q.get("n", [""])[0]
            if "/" not in full or not n.isdigit():
                self._html(400, error_page("Parâmetros inválidos", "Parâmetros inválidos."))
                return
            owner, repo = full.split("/", 1)
            try:
                open_pr(owner, repo, n)
            except Exception as e:  # noqa: BLE001 - mostra qualquer falha pro usuário
                self._html(502, error_page("Falha ao abrir", f"Falha ao abrir: {html.escape(str(e))}"))
                return
            self._redirect(BASE_PATH)
            return

        if u.path.startswith(BASE_PATH):
            self._proxy()
            return

        self._html(404, error_page("Não encontrado", "Não encontrado."))

    def do_POST(self):
        if not check_origin(self):
            self._html(403, error_page("Requisição inválida", "Origem não permitida (Origin mismatch)."))
            return

        u = urlparse(self.path)

        if u.path == "/login":
            form = parse_qs(self._body().decode("utf-8"))
            user = form.get("user", [""])[0]
            password = form.get("pass", [""])[0]
            login = check_forgejo_login(user, password) if user and password else None
            if not login:
                self._html(401, login_page(banner("err", "Usuário ou senha/token inválidos.")))
                return
            if not is_allowed(login):
                self._html(403, login_page(banner("err", f'Conta "{html.escape(login)}" não autorizada.')))
                return
            sid = new_session(login)
            self._redirect("/", {"Set-Cookie": f"{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Strict"})
            return

        session = session_from_cookie(self)
        if not session:
            self._redirect("/login")
            return

        if u.path == "/config/toml":
            form = parse_qs(self._body().decode("utf-8", errors="replace"))
            submitted_csrf = form.get("csrf", [""])[0]
            if not submitted_csrf or not secrets.compare_digest(submitted_csrf, session.get("csrf", "")):
                self._html(
                    403,
                    config_page(
                        session["user"],
                        toml_error=banner("err", "Token CSRF inválido ou ausente."),
                        csrf_token=session.get("csrf", ""),
                    ),
                )
                return
            new_content = form.get("toml", [""])[0]
            ok, outcome = check_candidate(new_content)
            if not ok:
                err = banner("err", "Configuração inválida, nada foi salvo:") + f'<pre class="mono text-xs bg-bg border border-border rounded-lg p-3 overflow-x-auto text-err mb-3">{html.escape(str(outcome))}</pre>'
                self._html(400, config_page(session["user"], toml_error=err, csrf_token=session.get("csrf", "")))
                return
            results = outcome
            if any(r.get("status") == "fail" for r in results):
                err = (
                    banner("err", "Checagem encontrou problema(s) bloqueante(s), nada foi salvo:")
                    + render_check_results(results)
                )
                self._html(400, config_page(session["user"], toml_error=err, csrf_token=session.get("csrf", "")))
                return
            try:
                write_file_atomic(TOML_PATH, new_content)
            except OSError as e:
                err = banner(
                    "err",
                    f"Validou, mas falhou ao escrever {html.escape(TOML_PATH)}: {html.escape(str(e))} — confira a "
                    "permissão do arquivo no host (<code>chmod 664 oreia-seca.toml</code>).",
                )
                self._html(502, config_page(session["user"], toml_error=err, csrf_token=session.get("csrf", "")))
                return
            restarted, msg = docker_restart("oreia-seca")
            note = banner("ok" if restarted else "err", f"oreia-seca.toml salvo. {html.escape(msg)}.") + render_check_results(results)
            self._html(200, config_page(session["user"], toml_ok=note, csrf_token=session.get("csrf", "")))
            return

        if u.path == "/config/env":
            form = parse_qs(self._body().decode("utf-8", errors="replace"))
            submitted_csrf = form.get("csrf", [""])[0]
            if not submitted_csrf or not secrets.compare_digest(submitted_csrf, session.get("csrf", "")):
                self._html(
                    403,
                    config_page(
                        session["user"],
                        env_error=banner("err", "Token CSRF inválido ou ausente."),
                        csrf_token=session.get("csrf", ""),
                    ),
                )
                return
            # Rejeita quebras de linha em qualquer campo submetido pra evitar injeção de novas variáveis
            for form_key, form_vals in form.items():
                for val in form_vals:
                    if "\n" in val or "\r" in val:
                        self._html(
                            400,
                            config_page(
                                session["user"],
                                env_error=banner("err", "Valor inválido: quebras de linha não são permitidas nos valores do .env."),
                                csrf_token=session.get("csrf", ""),
                            ),
                        )
                        return
            env_content = read_file_safe(ENV_PATH)
            if env_content is None:
                self._html(400, config_page(session["user"], env_error=banner("err", ".env não está montado."), csrf_token=session.get("csrf", "")))
                return
            updated_lines = []
            changed = 0
            for key, value in parse_env_lines(env_content):
                if key is None:
                    updated_lines.append(value)
                    continue
                form_key = f"env__{key}"
                is_secret = bool(SECRET_KEY_RE.search(key))
                if form_key in form:
                    new_value = form[form_key][0]
                    if is_secret:
                        # Vazio em campo de segredo = mantém o valor anterior (campo mascarado)
                        if new_value:
                            if new_value != value:
                                changed += 1
                            updated_lines.append(f"{key}={new_value}")
                        else:
                            updated_lines.append(f"{key}={value}")
                    else:
                        # Campo comum: permite esvaziar/remover valor
                        if new_value != value:
                            changed += 1
                        updated_lines.append(f"{key}={new_value}")
                else:
                    updated_lines.append(f"{key}={value}")
            try:
                write_file_atomic(ENV_PATH, "\n".join(updated_lines) + "\n")
            except OSError as e:
                err = banner(
                    "err",
                    f"Falha ao escrever {html.escape(ENV_PATH)}: {html.escape(str(e))} — confira a permissão do "
                    "arquivo no host (<code>chmod 664 .env</code>).",
                )
                self._html(502, config_page(session["user"], env_error=err, csrf_token=session.get("csrf", "")))
                return
            # Best-effort: o /api/check valida a FORMA do oreia-seca.toml atual (não mudou
            # aqui) e roda as checagens de rede contra o ambiente de ANTES do restart —
            # variáveis novas do .env só valem depois que o container reiniciar de verdade.
            # Não bloqueia salvar (diferente do /config/toml): só dá um retrato de antes.
            toml_now = read_file_safe(TOML_PATH) or ""
            check_ok, check_outcome = check_candidate(toml_now)
            check_html = render_check_results(check_outcome) if check_ok else ""

            ok1, msg1 = docker_restart("oreia-seca")
            note = (
                banner("ok", f".env salvo ({changed} campo(s) alterado(s)).")
                + banner("ok" if ok1 else "err", f"oreia-seca: {html.escape(msg1)}.")
                + check_html
                + '<div class="flex items-center gap-2 text-sm text-muted mt-1 mb-3">'
                '<span class="h-3 w-3 rounded-full border-2 border-accent border-t-transparent animate-spin"></span>'
                "px0 reiniciando pra aplicar as próprias variáveis — recarregue em alguns segundos e entre de novo.</div>"
            )
            self._html(200, config_page(session["user"], env_ok=note, csrf_token=session.get("csrf", "")))
            # Só depois de mandar a resposta: reiniciar o PRÓPRIO container no meio
            # da requisição mataria o processo antes de responder.
            threading.Timer(1.5, docker_restart, args=("px0",)).start()
            return

        if u.path.startswith(BASE_PATH):
            self._proxy()
            return

        self._html(404, error_page("Não encontrado", "Não encontrado."))

    # px0 usa outros métodos pra ações de git (stage, commit, push...)
    def do_PUT(self):
        self._auth_then_proxy()

    def do_DELETE(self):
        self._auth_then_proxy()

    def do_PATCH(self):
        self._auth_then_proxy()

    def _auth_then_proxy(self):
        if not session_from_cookie(self):
            self._redirect("/login")
            return
        if urlparse(self.path).path.startswith(BASE_PATH):
            self._proxy()
        else:
            self._html(404, error_page("Não encontrado", "Não encontrado."))

    def log_message(self, fmt, *args):  # silencia o access log padrão no stdout
        pass


if __name__ == "__main__":
    if not ALLOWED_USERS:
        print("px0-launcher: PX0_ALLOWED_USERS vazio, nenhuma conta poderá entrar", flush=True)
    setup_git_rewrite()
    threading.Thread(target=sample_memory_loop, daemon=True).start()
    ThreadingHTTPServer(("0.0.0.0", LAUNCHER_PORT), Handler).serve_forever()
