// src/models/local.js
import { BaseModel } from './base-model.js'

export class LocalModel extends BaseModel {
  /**
   * @param {object} opts
   * @param {string} opts.model — Nome do modelo local (ex: "qwen2.5-7b")
   * @param {string} opts.baseUrl — URL base da API compatível com OpenAI (ex: "http://localhost:3000/v1")
   * @param {string} [opts.apiKey] — Chave opcional (caso seja exigida pelo endpoint local)
   */
  constructor({ model = 'local-model', baseUrl = 'http://localhost:3000/v1', apiKey = 'none', context } = {}) {
    super()
    this.modelName = model
    this.baseUrl = baseUrl
    this.apiKey = apiKey
    this.context = context
    this._ready = false
  }

  // ─── Geração ─────────────────────────────────────────────────────────────

  async generate(messages, opts = {}) {
    const { system, maxTokens = 2048, temperature = 0.3 } = opts

    const fullMessages = system
      ? [{ role: 'system', content: system }, ...messages]
      : messages

    const url = `${this.baseUrl.replace(/\/$/, '')}/chat/completions`

    const res = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${this.apiKey}`,
      },
      body: JSON.stringify({
        model: this.modelName,
        messages: fullMessages.map(m => ({ role: m.role, content: m.content })),
        max_tokens: maxTokens,
        temperature,
      }),
      signal: AbortSignal.timeout(60_000), // 1 minuto max
    })

    if (!res.ok) {
      const errText = await res.text().catch(() => '')
      throw new Error(`Local API HTTP ${res.status}: ${errText.slice(0, 300)}`)
    }

    const data = await res.json()
    const text = data.choices?.[0]?.message?.content ?? ''

    return {
      text,
      usage: {
        prompt_tokens: data.usage?.prompt_tokens ?? 0,
        completion_tokens: data.usage?.completion_tokens ?? 0,
      }
    }
  }

  // ─── Inicialização ───────────────────────────────────────────────────────

  async init() {
    this._offline = false
    try {
      // Tenta pingar a lista de modelos para ver se o servidor local está online e respondendo
      const url = `${this.baseUrl.replace(/\/$/, '')}/models`
      const res = await fetch(url, {
        method: 'GET',
        headers: {
          'Authorization': `Bearer ${this.apiKey}`,
        },
        signal: AbortSignal.timeout(3_000), // timeout curto de 3s no boot
      })
      if (res.ok) {
        this._ready = true
        console.log(`[local] Conectado com sucesso ao provedor local no ${this.baseUrl} — modelo: ${this.modelName}`)
      } else {
        // Se retornar erro HTTP, mas o servidor respondeu, consideramos o endpoint de chat/completions pronto
        this._ready = true
        console.log(`[local] Provedor local no ${this.baseUrl} respondeu com HTTP ${res.status}. Assumindo pronto — modelo: ${this.modelName}`)
      }
    } catch (err) {
      console.warn(`[local] não foi possível conectar ao provedor local no ${this.baseUrl}: ${err.message}`)
      this.markOffline()
    }
  }

  // ─── Disponibilidade ─────────────────────────────────────────────────────

  isReady() {
    return this._ready && !this._offline
  }

  markOffline() {
    this._offline = true
    this._ready = false
  }

  async reload() {
    await this.init()
  }
}
