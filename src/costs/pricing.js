// src/costs/pricing.js

// Preços em USD por 1 milhão de tokens (fonte: páginas oficiais, maio 2025)
// Atualize conforme os provedores mudarem os preços
export const PRICING = {
  // ── Anthropic ──────────────────────────────────────────────────────────────
  'claude-opus-4-5':           { input: 15.00,  output: 75.00  },
  'claude-sonnet-4-5':         { input:  3.00,  output: 15.00  },
  'claude-haiku-4-5':          { input:  0.80,  output:  4.00  },

  // ── Google ─────────────────────────────────────────────────────────────────
  'gemini-2.5-pro':            { input:  1.25,  output: 10.00  },
  'gemini-2.5-flash':          { input:  0.075, output:  0.30  },
  'gemini-1.5-pro':            { input:  1.25,  output:  5.00  },
  'gemini-1.5-flash':          { input:  0.075, output:  0.30  },

  // ── OpenAI ─────────────────────────────────────────────────────────────────
  'gpt-4o':                    { input:  2.50,  output: 10.00  },
  'gpt-4o-mini':               { input:  0.15,  output:  0.60  },

  // ── Modelos locais — custo zero ────────────────────────────────────────────
  'lmstudio':                  { input:  0,     output:  0     },
  'ollama':                    { input:  0,     output:  0     },
}

// Retorna preço do modelo ou lança erro se desconhecido
export function getPrice(modelName) {
  // Match exato
  if (PRICING[modelName]) return PRICING[modelName]

  // Match parcial — ex: "claude-sonnet-4-5-20250514" → "claude-sonnet-4-5"
  const key = Object.keys(PRICING).find(k => modelName.startsWith(k))
  if (key) return PRICING[key]

  // Provider local — custo zero por padrão
  if (
    modelName === 'lmstudio' ||
    modelName === 'ollama' ||
    modelName === 'local' ||
    modelName.includes('local') ||
    modelName.includes('localhost')
  ) {
    return { input: 0, output: 0 }
  }

  throw new Error(
    `Preço não encontrado para o modelo "${modelName}".\n` +
    `Adicione em src/costs/pricing.js ou defina budget: null para desativar o controle.`
  )
}

// Calcula custo de uma chamada (em USD)
export function calcCost(usage, modelName) {
  const price = getPrice(modelName)
  const inputCost  = (usage.prompt_tokens     / 1_000_000) * price.input
  const outputCost = (usage.completion_tokens / 1_000_000) * price.output
  return inputCost + outputCost
}

// Formata valor em USD legível
export function formatUSD(amount) {
  if (amount < 0.001) return `$${(amount * 1000).toFixed(3)}m`  // milicents
  if (amount < 0.01)  return `$${amount.toFixed(4)}`
  if (amount < 1)     return `$${amount.toFixed(3)}`
  return `$${amount.toFixed(2)}`
}
