// src/maker/model-router.js
import { summarizeSession, saveSummaryFile } from './summarizer.js'

const DEBUG_TOKENS = process.argv.includes('--debug')

// ─── Barra de progresso de tokens ───────────────────────────────────────────

function tokenBar(used, limit, width = 40) {
  const ratio = Math.min(used / limit, 1)
  const pct   = (ratio * 100).toFixed(1)
  const filled = Math.round(ratio * width)
  const empty  = width - filled

  // Verde < 60% | Amarelo < 85% | Vermelho >= 85%
  const color = ratio < 0.60 ? '\x1b[32m'
              : ratio < 0.85 ? '\x1b[33m'
              :                '\x1b[31m'
  const reset = '\x1b[0m'

  const bar = color + '█'.repeat(filled) + reset + '░'.repeat(empty)
  return `         [${bar}] ${color}${pct}%${reset} (${used.toLocaleString()} / ${limit.toLocaleString()} tokens)`
}

// ─── Timeout helper ─────────────────────────────────────────────────────────

/**
 * Corre uma promise contra um rejeitor de timeout.
 * ms <= 0 desativa o timeout e retorna a promise original.
 */
function withTimeout(promise, ms, label) {
  if (!ms || ms <= 0) return promise
  let timer
  const race = Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => {
        const err = new Error(`Timeout de ${ms}ms atingido para "${label}"`)
        err.code = 'ETIMEOUT'
        reject(err)
      }, ms)
    })
  ])
  // Limpa o timer quando a promise original resolver primeiro
  promise.then(() => clearTimeout(timer), () => clearTimeout(timer))
  return race
}

/**
 * Gera um resumo rápido das mensagens usando o modelo de fallback.
 * Usado internamente após um timeout para preservar o contexto.
 */
async function summarizeForTimeout(messages, model, modelName) {
  // Extrai apenas as últimas 6 mensagens para o resumo ser rápido
  const slice = messages.slice(-6)
  try {
    return await summarizeSession(slice, model)
  } catch {
    // Fallback textual se o resumo também falhar
    return slice
      .map(m => `${m.role === 'assistant' ? 'Assistente' : 'Usuário'}: ${m.content.slice(0, 200)}`)
      .join('\n')
  }
}

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

  // Timeout por chamada: por modelo (context.timeout) ou global do router
  get _timeout() {
    return this._underlying().context?.timeout ?? this._router.modelTimeout ?? 0
  }

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
      const name = currentModel.modelName ?? currentModel.model ?? currentModel.constructor.name ?? '?'

      try {
        const timeoutMs = this._timeout
        const result = await withTimeout(
          currentModel.generate(messages, opts),
          timeoutMs,
          name
        )

        if (DEBUG_TOKENS) {
          const role  = this._role
          const pIn   = result.usage?.prompt_tokens     ?? 0
          const pOut  = result.usage?.completion_tokens ?? 0
          const total = pIn + pOut
          const limit = currentModel.context?.maxTokens ?? 8_192

          const inStr    = pIn.toLocaleString().padStart(6)
          const outStr   = pOut.toLocaleString().padStart(6)
          const totalStr = total.toLocaleString().padStart(6)

          console.log(`\x1b[36m[tokens]\x1b[0m papel=${role.padEnd(8)} modelo=${name}`)
          console.log(`         \x1b[2m↑ entrada:\x1b[0m ${inStr}  \x1b[2m↓ saída:\x1b[0m ${outStr}  \x1b[1mΣ total:\x1b[0m ${totalStr}`)
          console.log(tokenBar(total, limit))
        }
        return result

      } catch (err) {

        // ─── Timeout de modelo ──────────────────────────────────────────
        if (err.code === 'ETIMEOUT') {
          attempts++
          console.warn(`\n\x1b[33m[timeout]\x1b[0m ⏰ Modelo "${name}" não respondeu em ${this._timeout}ms.`)

          // Marca offline para o _resolve() avançar para o próximo
          currentModel.markOffline()

          const nextModel = this._underlying()
          const nextName  = nextModel.modelName ?? nextModel.model ?? nextModel.constructor.name ?? '?'
          const hasFallback = nextModel !== currentModel && nextModel.isReady()

          // Gera resumo do contexto com o próximo modelo (ou heurística se falhar)
          console.warn(`[timeout] 📝 Gerando resumo de contexto com "${hasFallback ? nextName : name}"...`)
          const summaryModel  = hasFallback ? nextModel : currentModel
          const summaryText   = await summarizeForTimeout(messages, summaryModel, name)
          const summaryFile   = await saveSummaryFile(`timeout-${this._role}`, summaryText).catch(() => null)
          if (summaryFile) {
            console.warn(`[timeout] 💾 Resumo salvo em: ${summaryFile}`)
          }

          // Reinicia o modelo original em background
          console.warn(`[timeout] 🔄 Reiniciando "${name}" em background...`)
          currentModel.reload?.().catch(e =>
            console.warn(`[timeout] reload de "${name}" falhou: ${e.message}`)
          )

          if (!hasFallback) {
            console.error(`[timeout] ❌ Sem modelo de fallback disponível para "${this._role}". Abortando.`)
            throw err
          }

          console.warn(`[timeout] ➡️  Continuando com "${nextName}".\n`)

          // Compacta messages: [resumo, última mensagem do usuário]
          const lastUserMsg = [...messages].reverse().find(m => m.role === 'user')
          messages = [
            { role: 'user', content: `📋 Contexto anterior (modelo anterior travou):\n${summaryText}` },
            ...(lastUserMsg ? [lastUserMsg] : [])
          ]
          continue
        }

        // ─── Erro de conexão ────────────────────────────────────────────
        if (_isConnectionError(err)) {
          attempts++
          console.warn(`\n[conexão] ⚠️ Conexão perdida com o modelo "${name}".`)
          console.warn(`[conexão] Detalhes do erro: ${err.message}`)

          currentModel.markOffline()

          const nextModel = this._underlying()
          const nextName  = nextModel.modelName ?? nextModel.model ?? nextModel.constructor.name

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
  constructor(roles, instances, fallbackChain = [], opts = {}) {
    /** Timeout global por chamada ao modelo (ms). 0 = sem limite. */
    this.modelTimeout = opts.modelTimeout ?? 0

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

  const modelTimeout = config.executor?.modelTimeout ?? 0
  if (modelTimeout > 0) {
    console.log(`[router] timeout por chamada ao modelo: ${modelTimeout}ms (sobrescrito por context.timeout por modelo)`)
  }
  const router = new ModelRouter(roles, instances, fallbackChain, { modelTimeout })
  await router.initAll()

  // Logs informativos das resoluções ativas por papel
  for (const { role, name, ready } of router.describe()) {
    const status = ready ? '✓' : '✖ (fallback/reserva)'
    console.log(`[router] papel ${role.padEnd(10)} → resolved a ${name} ${status}`)
  }

  return router
}
