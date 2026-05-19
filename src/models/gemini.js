// src/models/gemini.js
import { BaseModel } from './base-model.js'

export class GeminiModel extends BaseModel {
    /**
     * @param {object} opts
     * @param {string} opts.model   — Nome do modelo Gemini (ex: "gemini-2.5-flash")
     * @param {string} opts.apiKey  — Chave de API do Gemini
     */
    constructor({ model = 'gemini-2.5-flash', apiKey } = {}) {
        super()
        this.model = model
        this.apiKey = apiKey
    }

    // ─── Geração ─────────────────────────────────────────────────────────────

    async generate(messages, opts = {}) {
        if (!this.apiKey) {
            throw new Error('GeminiModel: apiKey é obrigatória. Configure GEMINI_KEY no seu arquivo .env')
        }

        const { system, maxTokens = 2048, temperature = 0.3 } = opts

        // Traduz o formato de mensagens [{ role: 'user'|'assistant', content }]
        // para o formato oficial do Gemini: [{ role: 'user'|'model', parts: [{ text }] }]
        const contents = messages.map(m => ({
            role: m.role === 'assistant' ? 'model' : 'user',
            parts: [{ text: m.content }]
        }))

        const body = {
            contents,
            generationConfig: {
                temperature,
                maxOutputTokens: maxTokens,
                ...(opts.json ? {
                    responseMimeType: 'application/json',
                    responseSchema: {
                        type: 'OBJECT',
                        properties: {
                            phases: {
                                type: 'ARRAY',
                                items: {
                                    type: 'OBJECT',
                                    properties: {
                                        id: { type: 'STRING' },
                                        name: { type: 'STRING' },
                                        instruction: { type: 'STRING' }
                                    },
                                    required: ['id', 'name', 'instruction']
                                }
                            }
                        },
                        required: ['phases']
                    }
                } : {}),
            }
        }

        if (system) {
            body.systemInstruction = {
                parts: [{ text: system }]
            }
        }

        const url = `https://generativelanguage.googleapis.com/v1beta/models/${this.model}:generateContent?key=${this.apiKey}`

        const res = await fetch(url, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
            signal: AbortSignal.timeout(60_000), // 1 minuto max
        })

        if (!res.ok) {
            const errText = await res.text().catch(() => '')
            throw new Error(`Gemini API HTTP ${res.status}: ${errText.slice(0, 300)}`)
        }

        const data = await res.json()
        const text = data.candidates?.[0]?.content?.parts?.[0]?.text ?? ''

        return {
            text,
            usage: {
                prompt_tokens: data.usageMetadata?.promptTokenCount ?? 0,
                completion_tokens: data.usageMetadata?.candidatesTokenCount ?? 0,
            }
        }
    }

    // ─── Inicialização ───────────────────────────────────────────────────────

    async init() {
        this.capabilities = {
            streaming: false,
            tokenize: false,
            reload: false,
            contextSize: 1_048_576, // ~1M tokens context window
        }

        if (!this.apiKey) {
            console.warn('[gemini] apiKey não configurada! O planejador Gemini falhará se for ativado.')
        } else {
            console.log(`[gemini] modelo ok — pronto para chamadas: ${this.model}`)
        }
    }
}
