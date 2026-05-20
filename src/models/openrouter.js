// src/models/openrouter.js
import { BaseModel } from './base-model.js'
import { OpenRouter } from '@openrouter/sdk'

export class OpenRouterModel extends BaseModel {
  /**
   * @param {object} opts
   * @param {string} opts.model  — ID do modelo no OpenRouter (ex: "anthropic/claude-sonnet-4")
   * @param {string} opts.apiKey — Chave de API do OpenRouter (sk-or-v1-...)
   */
  constructor({ model = 'openai/gpt-4.1-mini', apiKey } = {}) {
    super()
    this.model   = model
    this.apiKey  = apiKey
    this._client = null
  }

  // ─── Geração ─────────────────────────────────────────────────────────────

  async generate(messages, opts = {}) {
    if (!this._client) throw new Error('OpenRouter não inicializado (falta apiKey?)')

    const { system, maxTokens = 2048, temperature = 0.3 } = opts

    const fullMessages = system
      ? [{ role: 'system', content: system }, ...messages]
      : messages

    const completion = await this._client.chat.send({
      chatRequest: {
        model:       this.model,
        messages:    fullMessages,
        max_tokens:  maxTokens,
        temperature,
      },
    })

    const text = completion.choices?.[0]?.message?.content ?? ''

    return {
      text,
      usage: {
        prompt_tokens:     completion.usage?.promptTokens     ?? 0,
        completion_tokens: completion.usage?.completionTokens ?? 0,
      },
    }
  }

  // ─── Inicialização ───────────────────────────────────────────────────────

  async init() {
    if (!this.apiKey) {
      console.warn('[openrouter] apiKey não configurada! Configure OPENROUTER_KEY no .env')
      return
    }

    this._client = new OpenRouter({ apiKey: this.apiKey })
    console.log(`[openrouter] pronto — modelo: ${this.model}`)
  }

  // ─── Disponibilidade ──────────────────────────────────────────────────────

  isReady() {
    return this._client !== null
  }
}
