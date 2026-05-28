// src/maker/model-router.js

function _isConnectionError(err) {
  if (!err) return false

  const networkCodes = ['ECONNREFUSED', 'ECONNRESET', 'EPIPE', 'ETIMEDOUT', 'ENOTFOUND']
  if (err.code && networkCodes.includes(err.code)) {
    return true
  }

  const msg = (err.message || '').toLowerCase()
  const connectionSignals = [
    'websocket',
    'closed',
    'hang up',
    'not connected',
    'fetch failed',
    'connection refused',
    'connection reset',
    'connection error',
    'network error',
    'socket',
    'refused',
    'failed to fetch'
  ]

  return connectionSignals.some(sig => msg.includes(sig))
}

export class ResilientModel {
  constructor(router, role) {
    this._router = router
    this._role = role
  }

  _underlying() {
    return this._router._resolve(this._role)
  }

  get modelName() { return this._underlying().modelName }
  get model() { return this._underlying().model }
  get provider() { return this._underlying().provider }
  get apiKey() { return this._underlying().apiKey }
  get baseUrl() { return this._underlying().baseUrl }
  get capabilities() { return this._underlying().capabilities }
  get context() { return this._underlying().context }

  isReady() {
    return this._underlying().isReady()
  }

  markOffline() {
    this._underlying().markOffline()
  }

  async init() {
    return this._underlying().init()
  }

  async reload() {
    return this._underlying().reload()
  }

  async generate(messages, opts = {}) {
    let attempts = 0
    const maxAttempts = 3

    while (attempts < maxAttempts) {
      const currentModel = this._underlying()
      try {
        return await currentModel.generate(messages, opts)
      } catch (err) {
        if (_isConnectionError(err)) {
          attempts++
          const name = currentModel.modelName ?? currentModel.model ?? currentModel.constructor.name
          console.warn(`\n[conexão] ⚠️ Conexão perdida com o modelo "${name}".`)
          console.warn(`[conexão] Detalhes do erro: ${err.message}`)

          currentModel.markOffline()

          const nextModel = this._underlying()
          const nextName = nextModel.modelName ?? nextModel.model ?? nextModel.constructor.name

          if (nextModel === currentModel || !nextModel.isReady()) {
            console.error(`[conexão] ❌ Sem modelos de fallback/reserva disponíveis para o papel "${this._role}".`)
            throw err
          }

          console.warn(`[conexão] 🔄 Reconfigurando automaticamente para o modelo de reserva: "${nextName}"...\n`)
          continue
        }
        throw err
      }
    }
  }

  async countTokens(text) {
    let attempts = 0
    const maxAttempts = 3

    while (attempts < maxAttempts) {
      const currentModel = this._underlying()
      try {
        return await currentModel.countTokens(text)
      } catch (err) {
        if (_isConnectionError(err)) {
          attempts++
          currentModel.markOffline()

          const nextModel = this._underlying()
          if (nextModel === currentModel || !nextModel.isReady()) {
            throw err
          }

          console.warn(`[conexão] [tokens] 🔄 Fallback para contagem de tokens com: "${nextModel.modelName ?? nextModel.constructor.name}"`)
          continue
        }
        throw err
      }
    }
  }
}

/**
 * Roteia chamadas de modelo para provedores diferentes por etapa com cadeia de sucessão (titular/reserva).
 */
export class ModelRouter {
  /**
   * @param {object} roles              - Mapeamento de papéis para listas de chaves de modelos
   * @param {Map<string, BaseModel>} instances - Mapa de instâncias únicas de modelos
   * @param {string[]} fallbackChain    - Cadeia de fallback global
   */
  constructor(roles, instances, fallbackChain = []) {
    if (roles && !(roles instanceof Map) && !instances) {
      // Formato legado: constructor(models)
      const models = roles
      this.instances = new Map()
      this.roles = {}
      this.fallbackChain = []

      for (const [role, model] of Object.entries(models)) {
        this.instances.set(role, model)
        this.roles[role] = [role]
      }
      if (models.fallback) {
        this.instances.set('fallback', models.fallback)
        this.fallbackChain = ['fallback']
      }
      if (!this.roles.default && this.instances.has('default')) {
        this.roles.default = ['default']
      }
    } else {
      // Novo formato
      this.roles = roles
      this.instances = instances
      this.fallbackChain = fallbackChain
    }

    if (!this.roles.default && !this.roles.planner && !this.roles.executor) {
      throw new Error('ModelRouter: configuração de papéis inválida. É necessário ao menos um papel default/planner/executor.')
    }
  }

  // ─── Seletores Dinâmicos de Papéis Técnicos ───────────────────────────────

  forPlanning()   { return new ResilientModel(this, 'planner') }
  forExecution()  { return new ResilientModel(this, 'executor') }
  forDirect()     { return new ResilientModel(this, 'direct') }

  /**
   * Resolve o modelo ativo para um papel técnico usando a cadeia de sucessão:
   * 1. Papel específico (titular -> reservas)
   * 2. Default (titular -> reservas)
   * 3. Cadeia de Fallback Global
   */
  _resolve(role) {
    // 1. Tenta os modelos configurados para o papel específico
    const specificKeys = this.roles[role] || []
    for (const key of specificKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return model
    }

    // 2. Tenta os modelos configurados para o papel 'default'
    const defaultKeys = this.roles.default || []
    for (const key of defaultKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return model
    }

    // 3. Tenta a cadeia de fallback global de segurança
    const fallbackKeys = this.fallbackChain || []
    for (const key of fallbackKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return model
    }

    // Se tudo falhar, retorna a primeira tentativa do papel para gerar a falha de conexão na chamada
    const firstKey = specificKeys[0] || defaultKeys[0] || fallbackKeys[0]
    return this.instances.get(firstKey)
  }

  _resolveKey(role) {
    const specificKeys = this.roles[role] || []
    for (const key of specificKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return key
    }

    const defaultKeys = this.roles.default || []
    for (const key of defaultKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return key
    }

    const fallbackKeys = this.fallbackChain || []
    for (const key of fallbackKeys) {
      const model = this.instances.get(key)
      if (model?.isReady()) return key
    }

    return specificKeys[0] || defaultKeys[0] || fallbackKeys[0]
  }

  // ─── Inicializa todas as instâncias únicas de modelos ─────────────────────

  async initAll() {
    const unique = [...new Set(this.instances.values())]
    await Promise.all(unique.map(m => m.init?.()))
  }

  // ─── Lista as funções ativas e seus modelos resolvidos ───────────────────

  describe() {
    return Object.entries(this.roles).map(([role, keys]) => {
      const activeModel = this._resolve(role)
      return {
        role,
        name: activeModel ? (activeModel.modelName ?? activeModel.model ?? activeModel.constructor.name) : 'nenhum',
        ready: activeModel ? activeModel.isReady() : false,
      }
    })
  }
}

// ─── Factory: cria o router suportando nova e antiga estrutura ───────────────

export async function createRouter(config, modelFactory) {
  const instances = new Map()
  let roles = {}
  let fallbackChain = []

  if (Array.isArray(config.models)) {
    // ESTRUTURA UNIFICADA: Cadeia única de sucessão linear para todos os papéis
    const modelsChain = config.models
    roles = {
      default:    modelsChain,
      planner:    modelsChain,
      executor:   modelsChain,
      direct:     modelsChain
    }
    fallbackChain = []

    // Importa o registro desacoplado
    const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))

    for (const key of modelsChain) {
      const cfg = MODELS_REGISTRY[key]
      if (!cfg) {
        console.warn(`[router] aviso: especificação técnica de "${key}" não encontrada no registro`)
        continue
      }
      try {
        const instance = await modelFactory(cfg)
        instances.set(key, instance)
      } catch (err) {
        console.error(`[router] falha ao criar instância de "${key}":`, err.message)
      }
    }
  } else if (config.roles) {
    // NOVA ESTRUTURA: Papéis desacoplados e Cadeia de Sucessão
    roles = config.roles
    fallbackChain = config.fallbackChain || []

    const uniqueKeys = new Set([
      ...Object.values(roles).flat(),
      ...fallbackChain
    ])

    // Importa o registro desacoplado
    const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))

    for (const key of uniqueKeys) {
      const cfg = MODELS_REGISTRY[key] || config.models?.[key]
      if (!cfg) {
        console.warn(`[router] aviso: especificação técnica de "${key}" não encontrada no registro`)
        continue
      }
      try {
        const instance = await modelFactory(cfg)
        instances.set(key, instance)
      } catch (err) {
        console.error(`[router] falha ao criar instância de "${key}":`, err.message)
      }
    }
  } else {
    // RETROCOMPATIBILIDADE: Estrutura antiga (config.models e config.fallback)
    fallbackChain = config.fallback ? ['fallback'] : []

    if (config.fallback) {
      try {
        instances.set('fallback', await modelFactory(config.fallback))
      } catch (err) {
        console.warn(`[router] fallback legado ignorado: ${err.message}`)
      }
    }

    for (const [role, cfg] of Object.entries(config.models ?? {})) {
      try {
        const instance = await modelFactory(cfg)
        instances.set(role, instance)
        roles[role] = [role]
      } catch (err) {
        console.error(`[router] falha ao criar modelo legado "${role}":`, err.message)
      }
    }

    if (!roles.default && instances.has('default')) {
      roles.default = ['default']
    }
  }

  const router = new ModelRouter(roles, instances, fallbackChain)
  await router.initAll()

  // Logs informativos das resoluções ativas por papel
  for (const { role, name, ready } of router.describe()) {
    const status = ready ? '✓' : '✖ (fallback/reserva)'
    console.log(`[router] papel ${role.padEnd(10)} → resolved a ${name} ${status}`)
  }

  return router
}
