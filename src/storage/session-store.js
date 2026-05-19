// src/storage/session-store.js
import fs   from 'fs/promises'
import path from 'path'

const SESSIONS_DIR  = path.resolve('tasks', 'sessions')
const MAX_MESSAGES  = 20   // máximo de mensagens por sessão (10 trocas)
const MAX_AGE_MS    = 1000 * 60 * 60 * 2  // 2 horas — sessão expira se ficar inativa

class SessionStore {
  constructor() {
    // Mapa em memória: sessionKey → { messages, updatedAt }
    this._store = new Map()
  }

  // ─── Chave de sessão ───────────────────────────────────────────────────────
  // Formato: "source:userId" — ex: "telegram:12345", "terminal:local"

  _isExpired(session) {
    return Date.now() - new Date(session.updatedAt).getTime() > MAX_AGE_MS
  }

  // ─── Lê histórico da sessão ────────────────────────────────────────────────

  get(sessionKey) {
    const session = this._store.get(sessionKey)

    if (!session)              return []
    if (this._isExpired(session)) {
      this._store.delete(sessionKey)
      return []
    }

    return session.messages
  }

  // ─── Adiciona uma mensagem ─────────────────────────────────────────────────

  push(sessionKey, message) {
    const session = this._store.get(sessionKey) ?? {
      messages:  [],
      updatedAt: new Date().toISOString()
    }

    session.messages.push(message)
    session.updatedAt = new Date().toISOString()

    // Mantém só as últimas MAX_MESSAGES — descarta as mais antigas
    if (session.messages.length > MAX_MESSAGES) {
      // Sempre preserva a primeira mensagem (pode ter contexto importante)
      const first = session.messages[0]
      session.messages = [
        first,
        ...session.messages.slice(-(MAX_MESSAGES - 1))
      ]
    }

    this._store.set(sessionKey, session)
  }

  // ─── Reseta a sessão ───────────────────────────────────────────────────────

  clear(sessionKey) {
    this._store.delete(sessionKey)
  }

  // ─── Snapshot: injeta uma mensagem de sistema sem afetar o histórico ───────
  // Útil para passar o resultado de uma fase sem "poluir" a conversa

  snapshot(sessionKey, systemNote) {
    const messages = this.get(sessionKey)
    return [
      ...messages,
      { role: 'user', content: systemNote }
    ]
  }

  // ─── Info de debug ─────────────────────────────────────────────────────────

  info(sessionKey) {
    const session = this._store.get(sessionKey)
    if (!session) return { exists: false }
    return {
      exists:     true,
      messages:   session.messages.length,
      updatedAt:  session.updatedAt,
      expired:    this._isExpired(session),
    }
  }

  // ─── Persistência opcional em disco ────────────────────────────────────────
  // Chame saveAll() antes de encerrar o processo para sobreviver a restarts

  async saveAll() {
    await fs.mkdir(SESSIONS_DIR, { recursive: true })

    for (const [key, session] of this._store) {
      if (this._isExpired(session)) continue
      const file = path.join(SESSIONS_DIR, `${key.replace(':', '_')}.json`)
      await fs.writeFile(file, JSON.stringify(session, null, 2), 'utf-8')
        .catch(() => {})  // falha silenciosa — sessão não é crítica
    }
  }

  async loadAll() {
    try {
      await fs.mkdir(SESSIONS_DIR, { recursive: true })
      const files = await fs.readdir(SESSIONS_DIR)

      for (const file of files) {
        if (!file.endsWith('.json')) continue
        try {
          const raw     = await fs.readFile(path.join(SESSIONS_DIR, file), 'utf-8')
          const session = JSON.parse(raw)
          if (this._isExpired(session)) continue  // descarta expiradas

          // Reconstrói a chave: "telegram_12345.json" → "telegram:12345"
          const key = file.replace('.json', '').replace('_', ':')
          this._store.set(key, session)
        } catch {}
      }

      const loaded = this._store.size
      if (loaded > 0) console.log(`[sessions] ${loaded} sessão(ões) restaurada(s) do disco`)
    } catch {}
  }
}

export const sessionStore = new SessionStore()
