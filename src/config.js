// src/config.js

export const config = {
  // ─── Modelos por papel ──────────────────────────────────────────────────
  // O ModelRouter usa "default" como fallback quando um papel específico
  // não está configurado. Para usar um único modelo em tudo, defina só
  // "default". Para separar planner do executor, configure os dois.

  models: {
    default: {
      provider: 'gemini',
       name:     'gemini-3.5-flash',
       apiKey:   process.env.GEMINI_KEY,
    }, 
    
     executor: {
       provider: 'local',
       name:     'mimo-v2.5-pro',
       baseUrl:  'http://localhost:3000/v1',
       context: {
         maxTokens: 1_000_000, // Modelo local com 1M de tokens de contexto
         reserveOutput: 4_096  // Reserva 4K para geração de código completo
       }
     },

     planner: {
      provider: 'openrouter',
       name:     'nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free',
       apiKey:   process.env.OPENROUTER_KEY,
     },

     direct: {
       provider: 'local',
       name:     'mimo-v2.5-pro',
       baseUrl:  'http://localhost:3000/v1',
       context: {
         maxTokens: 1_000_000, // Modelo local com 1M de tokens de contexto
         reserveOutput: 4_096  // Reserva 4K para geração de código completo
       }
    },

    local: {
      provider: 'lmstudio',
      name: 'qwen/qwen3.5-9b',
      hfId: 'Qwen/Qwen2.5-7B', 
      baseUrl: process.env.LMSTUDIO_URL || 'http://localhost:1234',
        context: {
         maxTokens: 4_048, 
         reserveOutput: 900 
       }
     },
     

  },

  // ─── Modelo de fallback ─────────────────────────────────────────────────
  // Usado automaticamente quando um modelo configurado (ex: LM Studio)
  // não está acessível no momento do boot. Se o fallback também falhar,
  // o sistema não inicia.
  fallback: {
      provider: 'gemini',
       name:     'gemini-3.5-flash',
       apiKey:   process.env.GEMINI_KEY,
  },

  // ─── Fila de tarefas ────────────────────────────────────────────────────
  queue: {
    concurrency: 1,         // 1 = serial (obrigatório para modelo local)
    timeout: 0,         // ms por tarefa, 0 = sem limite
  },

  // ─── Canais de entrada ──────────────────────────────────────────────────
  inputs: [
    { type: 'terminal', enabled: true },
    {
      type: 'telegram',
      enabled: !!process.env.TELEGRAM_TOKEN,
      token: process.env.TELEGRAM_TOKEN,
      allowedUsers: (process.env.TELEGRAM_ALLOWED ?? '').split(',').filter(Boolean),
    },
    {
      type: 'rest',
      enabled: true,
      port: Number(process.env.REST_PORT) || 3120,
      apiKey: process.env.REST_API_KEY || null,   // null = sem autenticação
    },
  ],

  // ─── Context window ─────────────────────────────────────────────────────
  // Ajuste conforme a configuração do modelo no LM Studio
  context: {
    maxTokens: 8_192,   // Qwen3.5-9B: até 128K — use o que o LM Studio aceitar
    reserveOutput: 2_048,   // tokens reservados para o modelo gerar
  },

  // ─── Executor ───────────────────────────────────────────────────────────
  executor: {
    maxIterations: 20,
    phaseTimeout: 120_000,
    reloadDelay: 2_000,
  },
}
