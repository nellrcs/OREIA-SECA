// src/maker/model-router.js

/**
 * Roteia chamadas de modelo para provedores diferentes por etapa.
 *
 * Casos de uso:
 *   - Planner: modelo rápido/barato (qwen 4B local)
 *   - Executor: modelo mais capaz (Gemini Pro ou 7B local)
 *   - Direto: modelo rápido (mesmo do planner)
 *
 * Se um modelo específico não estiver configurado, cai no `default`.
 */
export class ModelRouter {
  /**
   * @param {object} models
   * @param {BaseModel} models.default   — usado quando nenhum específico existe
   * @param {BaseModel} [models.planner] — chamadas do planner
   * @param {BaseModel} [models.executor]— chamadas do executor por fase
   * @param {BaseModel} [models.direct]  — respostas diretas (sem fases)
   */
  constructor(models) {
    if (!models.default) throw new Error('ModelRouter: models.default é obrigatório')
    this.models = models
  }

  // ─── Seletores ────────────────────────────────────────────────────────────

  forPlanning()   { return this.models.planner  ?? this.models.default }
  forExecution()  { return this.models.executor ?? this.models.default }
  forDirect()     { return this.models.direct   ?? this.models.default }

  // ─── Inicializa todos os modelos configurados ─────────────────────────────

  async initAll() {
    const unique = [...new Set(Object.values(this.models))]
    await Promise.all(unique.map(m => m.init?.()))
  }

  // ─── Lista os modelos e papéis ────────────────────────────────────────────

  describe() {
    return Object.entries(this.models).map(([role, model]) => ({
      role,
      name: model.model ?? model.constructor.name,
    }))
  }
}

// ─── Factory: cria o router a partir do config ────────────────────────────────

export async function createRouter(config, modelFactory) {
  const models = {}

  for (const [role, cfg] of Object.entries(config.models ?? {})) {
    models[role] = await modelFactory(cfg)
  }

  // Garante que sempre existe um default
  if (!models.default) {
    throw new Error('config.models.default é obrigatório')
  }

  const router = new ModelRouter(models)
  await router.initAll()

  for (const { role, name } of router.describe()) {
    console.log(`[router] ${role.padEnd(8)} → ${name}`)
  }

  return router
}
