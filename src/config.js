// src/config.js

export const config = {
  // ─── Modelos por papel ──────────────────────────────────────────────────
  // O ModelRouter usa "default" como fallback quando um papel específico
  // não está configurado. Para usar um único modelo em tudo, defina só
  // "default". Para separar planner do executor, configure os dois.

  models: {
    default: {
      provider: 'lmstudio',
      name: 'qwen/qwen3.5-9b',
      hfId: 'Qwen/Qwen2.5-7B',   // tokenizer compatível no HuggingFace
      baseUrl: 'http://localhost:1234',
    },

    // Descomente para usar um modelo mais capaz só nas execuções de fase:
    // executor: {
    //   provider: 'lmstudio',
    //   name:     'qwen2.5-coder-7b',
    //   baseUrl:  'http://localhost:1234',
    // },

    // Descomente para planejar com Gemini e executar localmente:
    // planner: {
    //   provider: 'gemini',
    //   name:     'gemini-2.5-flash',
    //   apiKey:   process.env.GEMINI_KEY,
    // },
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
