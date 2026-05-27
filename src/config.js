// src/config.js

export const config = {
  // ─── Modelos por papel ──────────────────────────────────────────────────
  // O ModelRouter usa "default" como fallback quando um papel específico
  // não está configurado. Para usar um único modelo em tudo, defina só
  // "default". Para separar planner do executor, configure os dois.

  // ─── Papéis Técnicos de Agentes (com Cadeia de Sucessão) ───────────────────
  // Para cada papel técnico, defina uma lista priorizada de modelos cadastrados.
  // O sistema usará o titular (primeiro) e usará os reservas se o titular estiver offline.
  roles: {
    // Planejador de tarefas e criador das etapas
    planner:    ['nemotron-free'],

    // Pesquisador de informações e análise do codebase
    researcher: ['qwen-lmstudio','mimo-local'],

    // Agente executor que roda as ações de cada fase
    executor:   ['qwen-lmstudio', 'nemotron-free'],

    // Validador e homologador de resultados técnicos de cada fase
    validator:  ['nemotron-free'],

    // Respostas normais e rápidas diretas no chat
    direct:  ['qwen-lmstudio','mimo-local','gemini-flash'],

    // Fallback padrão caso um papel específico não possua modelos definidos
    default:    ['qwen-lmstudio']
  },

  // ─── Cadeia de Fallback Global de Segurança ──────────────────────────────
  // Ativado automaticamente como última linha de defesa se nenhum modelo do papel
  // ou do 'default' responder no boot do sistema.
  fallbackChain: ['gemini-flash', 'nemotron-free'],

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
