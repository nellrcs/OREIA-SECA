// src/maker/model-router.js

/**
 * Roteia chamadas de modelo para provedores diferentes por etapa.
 *
 * Casos de uso:
 *   - Planner: modelo rápido/barato (qwen 4B local)
 *   - Executor: modelo mais capaz (Gemini Pro ou 7B local)
 *   - Direto: modelo rápido (mesmo do planner)
 *
 * Hierarquia:
 *   1. Usa o modelo configurado para o papel (planner, executor, direct)
 *   2. Se não configurado, usa o modelo "default"
 *   3. Se o modelo não estiver pronto (isReady() = false), usa o fallback
 */
export class ModelRouter {
  /**
   * @param {object} models
   * @param {BaseModel} models.default    — usado quando nenhum específico existe
   * @param {BaseModel} [models.planner]  — chamadas do planner
   * @param {BaseModel} [models.executor] — chamadas do executor por fase
   * @param {BaseModel} [models.direct]   — respostas diretas (sem fases)
   * @param {BaseModel} [models.fallback] — substituto automático quando modelo falha
   */
  constructor(models) {
    if (!models.default) throw new Error('ModelRouter: models.default é obrigatório')
    this.models = models
  }

  // ─── Seletores (com fallback automático) ──────────────────────────────────

  forPlanning()  { return this._resolve('planner') }
  forExecution() { return this._resolve('executor') }
  forDirect()    { return this._resolve('direct') }

  /**
   * Resolve o modelo para um papel, com cadeia de fallback:
   *   papel específico → default → fallback
   */
  _resolve(role) {
    const specific = this.models[role]
    if (specific?.isReady()) return specific

    const def = this.models.default
    if (def?.isReady()) return def

    const fb = this.models.fallback
    if (fb?.isReady()) return fb

    // Se nada está pronto, retorna o default mesmo (vai falhar na chamada com mensagem clara)
    return def
  }

  // ─── Inicializa todos os modelos configurados ─────────────────────────────

  async initAll() {
    const unique = [...new Set(Object.values(this.models))]
    await Promise.all(unique.map(m => m.init?.()))
  }

  // ─── Lista os modelos e papéis ────────────────────────────────────────────

  describe() {
    return Object.entries(this.models).map(([role, model]) => ({
      role,
      name: model.modelName ?? model.model ?? model.constructor.name,
      ready: model.isReady(),
    }))
  }
}

// ─── Factory: cria o router a partir do config ────────────────────────────────

export async function createRouter(config, modelFactory) {
  const models = {}

  // 1. Cria todos os modelos configurados por papel
  for (const [role, cfg] of Object.entries(config.models ?? {})) {
    models[role] = await modelFactory(cfg)
  }

  // 2. Cria o modelo fallback (se configurado)
  if (config.fallback) {
    try {
      models.fallback = await modelFactory(config.fallback)
    } catch (err) {
      console.warn(`[router] fallback ignorado: ${err.message}`)
    }
  }

  // 3. Garante que sempre existe um default
  if (!models.default) {
    throw new Error('config.models.default é obrigatório')
  }

  // 4. Inicializa todos
  const router = new ModelRouter(models)
  await router.initAll()

  // 5. Verifica disponibilidade e loga substituições
  const defaultReady = models.default.isReady()

  if (!defaultReady && models.fallback?.isReady()) {
    console.warn('[router] ⚠ modelo default indisponível — usando fallback para todos os papéis')
  } else if (!defaultReady && !models.fallback?.isReady()) {
    console.error('[router] ✖ modelo default E fallback indisponíveis!')
    throw new Error(
      'Nenhum modelo disponível. Inicie o LM Studio ou configure GEMINI_KEY no .env'
    )
  }

  for (const { role, name, ready } of router.describe()) {
    const status = ready ? '✓' : '✖ (fallback)'
    console.log(`[router] ${role.padEnd(9)} → ${name} ${status}`)
  }

  return router
}
