// src/models/lmstudio.js
import { BaseModel } from './base-model.js'
import { LMStudioClient } from '@lmstudio/sdk'

const sleep = ms => new Promise(r => setTimeout(r, ms))

export class LMStudioModel extends BaseModel {
  constructor({ model, baseUrl = 'ws://127.0.0.1:1234' } = {}) {
    super()
    this.modelName = model
    this.baseUrl = baseUrl.replace(/^http/, 'ws') // O SDK exige ws:// ou lida com ele nativamente
    this.client = new LMStudioClient({ baseUrl: this.baseUrl })
    this.model = null
  }

  // ─── Geração ─────────────────────────────────────────────────────────────

  async generate(messages, opts = {}) {
    if (!this.model) throw new Error('Modelo LM Studio não está carregado (via SDK)')

    const { system, maxTokens = 4024, temperature = 0.3 } = opts

    const fullMessages = system
      ? [{ role: 'system', content: system }, ...messages]
      : messages

    const result = await this.model.respond(fullMessages, {
      maxTokens,
      temperature,
    })

    return {
      text: result.content,
      usage: {
        prompt_tokens: result.stats?.promptTokensCount ?? 0,
        completion_tokens: result.stats?.predictedTokensCount ?? 0,
      },
    }
  }

  // ─── Contagem de tokens ──────────────────────────────────────────────────

  async countTokens(text) {
    if (!this.model) return this._heuristic(text)
    try {
      const tokens = await this.model.tokenize(text)
      return tokens.length
    } catch {
      return this._heuristic(text)
    }
  }

  // ─── Reload ──────────────────────────────────────────────────────────────

  async reload() {
    this._offline = false
    try {
      if (this.model) {
        await this.model.unload().catch(() => {})
      }
      await sleep(1_500)
      this.model = await this.client.llm.model(this.modelName)
      console.log(`[lmstudio] SDK modelo recarregado: ${this.modelName}`)
    } catch (err) {
      console.warn(`[lmstudio] SDK reload falhou: ${err.message}`)
      this.model = null
      this.markOffline()
    }
  }

  // ─── Init — conecta ao LM Studio via SDK ──────────────────────────────

  async init() {
    try {
      this._offline = false
      this.model = await this.client.llm.model(this.modelName)
      console.log(`[lmstudio] SDK conectado — modelo carregado: ${this.modelName}`)
    } catch (err) {
      console.warn(`[lmstudio] erro ao carregar ${this.modelName} via SDK: ${err.message}`)
      console.warn('[lmstudio] inicie o LM Studio e garanta que o lms-server está rodando')
      this.model = null
      this.markOffline()
    }
  }

  // ─── Disponibilidade ──────────────────────────────────────────────────

  isReady() {
    return this.model !== null && !this._offline
  }

  markOffline() {
    this._offline = true
    this.model = null
  }

  // ─── Heurística de fallback ───────────────────────────────────────────────

  _heuristic(text) {
    // ~4 chars/token com margem de 15%
    return Math.ceil((text.length / 4) * 1.15)
  }
}
