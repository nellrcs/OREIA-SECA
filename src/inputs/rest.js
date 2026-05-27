// src/inputs/rest.js
import http from 'http'
import { BaseInput } from './base-input.js'

export class RestInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._server = null
    this._port   = cfg.port ?? 3120
    this._apiKey = cfg.apiKey ?? null   // proteção opcional
    this._log    = []                    // últimas respostas (para consulta via GET)
    this._telegramInput = null
  }

  setTelegramInput(telegramInput) {
    this._telegramInput = telegramInput
  }

  async start() {
    this._server = http.createServer((req, res) => this._route(req, res))

    return new Promise((resolve, reject) => {
      this._server.listen(this._port, () => {
        console.log(`[rest] servidor HTTP em http://localhost:${this._port}`)
        console.log(`[rest] endpoints:`)
        console.log(`       POST /api/message   — envia mensagem ao agente`)
        console.log(`       GET  /api/status    — status da fila`)
        console.log(`       GET  /api/log       — últimas respostas`)
        resolve()
      })
      this._server.on('error', reject)
    })
  }

  async stop() {
    return new Promise(resolve => {
      this._server?.close(() => resolve())
    })
  }

  // ─── Roteamento HTTP ──────────────────────────────────────────────────────

  async _route(req, res) {
    // CORS para chamadas locais
    res.setHeader('Access-Control-Allow-Origin', '*')
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization')
    if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return }

    const url = new URL(req.url, `http://localhost:${this._port}`)

    // Autenticação opcional
    if (this._apiKey) {
      const auth = req.headers['authorization'] ?? ''
      if (auth !== `Bearer ${this._apiKey}`) {
        await this._readBody(req).catch(() => {}) // consome todo o corpo para evitar ECONNRESET
        return this._json(res, 401, { error: 'Unauthorized' })
      }
    }

    try {
      if (req.method === 'POST' && url.pathname === '/api/message') {
        return await this._handleMessage(req, res)
      }
      if (req.method === 'GET' && url.pathname === '/api/status') {
        return await this._handleStatus(req, res)
      }
      if (req.method === 'GET' && url.pathname === '/api/log') {
        return this._json(res, 200, { log: this._log.slice(-20) })
      }
      this._json(res, 404, { error: 'Not found' })
    } catch (err) {
      console.error('[rest] erro:', err.message)
      this._json(res, 500, { error: err.message })
    }
  }

  // ─── POST /api/message ────────────────────────────────────────────────────

  async _handleMessage(req, res) {
    const body = await this._readBody(req)
    const { text, userId } = JSON.parse(body)

    if (!text) return this._json(res, 400, { error: 'Campo "text" obrigatório' })

    const msgUserId = userId ?? 'rest-api'

    // Responde 202 imediatamente — a mensagem será processada em background
    this._json(res, 202, {
      status:  'accepted',
      message: `Mensagem recebida: "${text.slice(0, 60)}"`,
      userId:  msgUserId,
    })

    // Retransmite a requisição para o Telegram se configurado
    if (this._telegramInput) {
      await this._telegramInput.send('broadcast', `📥 *[API]* Nova requisição recebida de \`${msgUserId}\`:\n"${text}"`).catch(err => {
        console.error(`[rest] erro ao retransmitir requisição para Telegram:`, err.message)
      })
    }

    // Dispara para o Maker em background
    this._handler?.({
      source:   'rest',
      userId:   msgUserId,
      text,
      metadata: {},
    })
  }

  // ─── GET /api/status ──────────────────────────────────────────────────────

  async _handleStatus(req, res) {
    // O Maker injeta status via send()
    this._json(res, 200, {
      online: true,
      log:    this._log.slice(-5),
    })
  }

  // ─── Envio de respostas (armazena no log) ─────────────────────────────────

  async send(userId, text) {
    const entry = {
      timestamp: new Date().toISOString(),
      userId,
      text: text.slice(0, 500),
    }
    this._log.push(entry)
    if (this._log.length > 100) this._log.shift()

    console.log(`[rest] resposta → ${userId}: ${text.slice(0, 80)}`)

    // Retransmite a resposta para o Telegram se configurado
    if (this._telegramInput) {
      await this._telegramInput.send('broadcast', `📢 *[API]* Resposta para \`${userId}\`:\n${text}`).catch(err => {
        console.error(`[rest] erro ao retransmitir resposta para Telegram:`, err.message)
      })
    }
  }

  async sendTyping(_userId) {
    // REST não tem conceito de "digitando"
  }

  // ─── Helpers ──────────────────────────────────────────────────────────────

  _json(res, status, data) {
    res.writeHead(status, {
      'Content-Type': 'application/json',
      'Connection': 'close'
    })
    res.end(JSON.stringify(data))
  }

  _readBody(req) {
    return new Promise((resolve, reject) => {
      const chunks = []
      req.on('data', c => chunks.push(c))
      req.on('end', () => resolve(Buffer.concat(chunks).toString()))
      req.on('error', reject)
    })
  }
}
