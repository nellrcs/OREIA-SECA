// src/models/base-model.js
export class BaseModel {
  // messages = [{ role: 'user'|'assistant', content: string }]
  // opts = { system?, maxTokens?, temperature? }
  // → { text: string, usage: { prompt_tokens, completion_tokens } }
  async generate(messages, opts = {}) {
    throw new Error(`${this.constructor.name}: generate() não implementado`)
  }

  // Estimativa de tokens (pode ser sobrescrito com contagem real)
  async countTokens(text) {
    return Math.ceil(text.length / 4)
  }

  // Libera e recarrega o modelo (implementado só em LMStudio)
  async reload() {}

  // Inicialização assíncrona opcional
  async init() {}
}
