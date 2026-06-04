// src/config/models-registry.js

export const MODELS_REGISTRY = {
// ─── Provedores Locais ─────────────────────────────────────────────────────
'mimo-local': {
  provider: 'local',
  name:     'mimo-v2.5-pro',
  baseUrl:  'http://192.168.2.101:3000/v1',
  context: {
    maxTokens: 1_000_000, // 1M de contexto
    reserveOutput: 4_096
  }
},

'qwen-lmstudio': {
  provider: 'lmstudio',
  name:     'qwen/qwen3.5-9b',
  baseUrl:  process.env.LMSTUDIO_URL || 'http://localhost:1234',
  hfId:     'Qwen/Qwen2.5-7B',
  context: {
    maxTokens: 8_048, // Qwen 2.5 geralmente suporta até 128K
    reserveOutput: 900,
    timeout: 5_000  
  }
},

// ─── APIs na Nuvem ────────────────────────────────────────────────────────
'gemini-flash': {
  provider: 'gemini',
  name:     'gemini-2.5-flash',
  apiKey:   process.env.GEMINI_KEY,
  context: {
    maxTokens: 1_048_576, // Gemini Flash suporta ~1M
    reserveOutput: 8_192
  }
},

'nemotron-free': {
  provider: 'openrouter',
  name:     'nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free',
  apiKey:   process.env.OPENROUTER_KEY,
  context: {
    maxTokens: 128_000, // valor seguro para OpenRouter/free
    reserveOutput: 4_096
  }
},
'deepseek-v4-flash': {
  provider: 'openrouter',
  name:     'deepseek/deepseek-v4-flash:free',
  apiKey:   process.env.OPENROUTER_KEY,
  context: {
    maxTokens: 128_000, // valor seguro para OpenRouter/free
    reserveOutput: 4_096
  }
}

}
