// src/maker/token-counter.js
import { LMStudioClient } from '@lmstudio/sdk'

const CHARS_PER_TOKEN = {
  llama:   { en: 4.0, pt: 3.2, code: 3.5 },
  mistral: { en: 4.0, pt: 3.2, code: 3.5 },
  qwen:    { en: 4.5, pt: 3.6, code: 4.0 },
  phi:     { en: 4.2, pt: 3.4, code: 3.8 },
  default: { en: 4.0, pt: 3.0, code: 3.5 },
}

function detectFamily(modelName = '') {
  const n = modelName.toLowerCase()
  if (n.includes('llama'))   return 'llama'
  if (n.includes('mistral')) return 'mistral'
  if (n.includes('qwen'))    return 'qwen'
  if (n.includes('phi'))     return 'phi'
  return 'default'
}

function detectContentType(text) {
  const codeSignals = (text.match(/[{}();=><]/g) || []).length
  const ptSignals   = (text.match(/[ãõçáéíóúâêôà]/g) || []).length
  if (codeSignals > text.length * 0.04) return 'code'
  if (ptSignals   > text.length * 0.02) return 'pt'
  return 'en'
}

function estimateTokens(text, modelName) {
  const family = detectFamily(modelName)
  const type   = detectContentType(text)
  const ratio  = CHARS_PER_TOKEN[family][type]
  return Math.ceil((text.length / ratio) * 1.15)
}

export class TokenCounter {
  constructor({ lmStudioUrl, modelId, modelName } = {}) {
    this.lmStudioUrl = lmStudioUrl ? lmStudioUrl.replace(/^http/, 'ws') : null
    this.modelId     = modelId
    this.modelName   = modelName
    this.strategy    = 'heuristic'
    this.lmsModel    = null
  }

  async init() {
    // Tenta LM Studio SDK primeiro
    if (this.lmStudioUrl) {
      try {
        const client = new LMStudioClient({ baseUrl: this.lmStudioUrl })
        this.lmsModel = await client.llm.model(this.modelName)
        this.strategy = 'lmstudio'
        console.log('[tokens] usando LM Studio SDK')
        return
      } catch (err) {
        console.warn(`[tokens] LM Studio SDK falhou (${err.message}) - tentando fallback...`)
      }
    }

    // Tenta tokenizer HuggingFace
    if (this.modelId) {
      try {
        const { AutoTokenizer } = await import('@xenova/transformers')
        this.tokenizer = await AutoTokenizer.from_pretrained(this.modelId)
        this.strategy  = 'xenova'
        console.log('[tokens] usando tokenizer HuggingFace:', this.modelId)
        return
      } catch {}
    }

    console.log(`[tokens] usando heurística calibrada (${detectFamily(this.modelName)})`)
  }

  async count(text, forceStrategy) {
    const strat = forceStrategy ?? this.strategy

    if (strat === 'lmstudio' && this.lmsModel) {
      try {
        const tokens = await this.lmsModel.tokenize(text)
        return tokens.length
      } catch {
        // Fallback silently
      }
    }

    if (strat === 'xenova' && this.tokenizer) {
      return this.tokenizer.encode(text).length
    }

    return estimateTokens(text, this.modelName)
  }

  async willFit(messages, maxTokens = 2048, reserveForOutput = 900) {
    const budget = maxTokens - reserveForOutput
    const counts = await Promise.all(
      messages.map(m => this.count(`${m.role}: ${m.content}`))
    )
    const used = counts.reduce((a, b) => a + b, 0)
    return { fits: used <= budget, used, budget }
  }
}
