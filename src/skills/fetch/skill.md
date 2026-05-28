# skill_fetch

Fetches raw text or HTML content from a public URL via HTTP request.

## Parameters
- `url` (string, required): The absolute URL to request (e.g., https://api.github.com).
- `method` (string, optional): HTTP method to use (GET, POST). Defaults to GET.

## Code
```javascript
// Validação de segurança de rede local
let parsed
try {
  parsed = new URL(url)
} catch (err) {
  throw new Error(`skill_fetch: invalid URL: ${url}`)
}

const blockedHosts = ['localhost', '127.0.0.1', '0.0.0.0', 'host.docker.internal']
if (blockedHosts.includes(parsed.hostname) || parsed.hostname.startsWith('192.168.') || parsed.hostname.startsWith('10.')) {
  throw new Error('skill_fetch: requesting private/local network addresses is blocked for security reasons')
}

const controller = new AbortController()
const id = setTimeout(() => controller.abort(), 10000)

try {
  const res = await fetch(url, {
    method: method || 'GET',
    signal: controller.signal,
    headers: {
      'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) OREIASECA/0.1.0'
    }
  })
  clearTimeout(id)

  if (!res.ok) {
    throw new Error(`HTTP ${res.status} ${res.statusText}`)
  }

  const text = await res.text()
  // Retorna uma versão truncada para evitar estourar a janela de contexto da LLM
  return `[HTTP ${res.status}] Response from ${url} (${text.length} bytes):\n${text.slice(0, 1500)}${text.length > 1500 ? '\n... (truncated)' : ''}`
} catch (err) {
  clearTimeout(id)
  throw new Error(`skill_fetch failed: ${err.message}`)
}
```
