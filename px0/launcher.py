#!/usr/bin/env python3
"""Página intermediária: autentica com as MESMAS credenciais do Forgejo (conta
ou token pessoal), lista PRs abertos, e ao clicar "Revisar" lança o px0 pra
esse PR e faz proxy reverso dele sob a MESMA sessão — o px0 em si não tem
login nenhum, então é essa camada que o protege. Veja docs/px0.md.

Só usa a biblioteca padrão de propósito (sem dependências pra instalar).
"""
import base64
import html
import http.client
import json
import os
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request
from http import cookies
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, urlencode, urlparse

FORGEJO_API = os.environ.get("FORGEJO_API", "http://forgejo:3000/api/v1")
FORGEJO_INTERNAL = "http://forgejo:3000"  # chamadas servidor-a-servidor (token exchange, userinfo)
GITEA_TOKEN = os.environ.get("GITEA_TOKEN", "")
GITEA_HOST = os.environ.get("GITEA_HOST", "forgejo-tls:8443")
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
    sessions[sid] = {"user": user, "expires": time.time() + SESSION_TTL}
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
    if current.get("log_file"):
        try:
            current["log_file"].close()
        except OSError:
            pass

    url = f"http://forgejo-tls:8443/{owner}/{repo}/pulls/{n}"
    env = os.environ.copy()
    env.setdefault("GITEA_HOST", GITEA_HOST)
    if GITEA_TOKEN:
        env.setdefault("GITEA_TOKEN", GITEA_TOKEN)

    log_path = "/tmp/px0-session.log"
    log_f = open(log_path, "w+", encoding="utf-8", errors="replace")
    proc = subprocess.Popen(
        ["/usr/local/bin/px0", "-host", "127.0.0.1", "-port", str(REVIEW_PORT), "-no-open", "-base-path", BASE_PATH, url],
        stdout=log_f,
        stderr=subprocess.STDOUT,
        env=env,
    )
    current["proc"] = proc
    current["key"] = key
    current["log_file"] = log_f
    for _ in range(60):
        if port_open(REVIEW_PORT):
            return
        if proc.poll() is not None:
            log_f.seek(0)
            err_details = log_f.read().strip()
            print(f"px0-launcher: px0 encerrou prematuramente:\n{err_details}", flush=True)
            msg = "px0 saiu antes de abrir a porta"
            if err_details:
                # pega a última linha não-vazia para um resumo claro
                lines = [l.strip() for l in err_details.splitlines() if l.strip()]
                msg = f"{msg}: {lines[-1]}"
            raise RuntimeError(msg)
        time.sleep(0.5)
    raise RuntimeError("px0 não respondeu a tempo")


# ── Páginas ───────────────────────────────────────────────────────────────────
# Visual inspirado no https://forgeui.in/ — mas sem adotar os componentes React
# dele: este launcher é um único arquivo Python (biblioteca padrão, sem
# Node/build), e trocar a arquitetura toda por um front-end React só pra ter
# esse visual seria desproporcional. Em vez disso: as cores SÃO as mesmas (os
# tokens OKLCH do tema escuro padrão shadcn/ui — "neutral", extraídos de
# verdade do CSS compilado de forgeui.in, não um palpite) + Tailwind via CDN
# (sem build) + um fundo com glow radial/grade sutil (o mesmo tipo de
# `bg-gradient-to-*`/`blur-*` que o bundle deles usa) + animações CSS/Tailwind.
# Mesmo espírito visual, zero dependência de Node. Veja docs/px0.md.

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
      <p class="text-muted text-sm mt-1">Revisão de PR</p>
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
        # Status e Configuração saíram daqui — agora vivem em `oreia-seca
        # setup` (áreas Status/MCP/Forgejo/Servidor na TUI). Veja docs/px0.md.
        links = [("/", "PRs")]
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
    ThreadingHTTPServer(("0.0.0.0", LAUNCHER_PORT), Handler).serve_forever()
