// src/config.js

export const config = {
  // ─── Modelos do Sistema (com Cadeia de Sucessão) ───────────────────
  // O principal modelo ativo e os backups em ordem de sucessão caso o titular esteja offline.
  models: ['qwen-lmstudio', 'mimo-local', 'gemini-flash', 'nemotron-free'],

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
