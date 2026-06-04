// src/maker/index.js
import crypto             from 'crypto'
import { planTask }       from './planner.js'
import { executeTask }    from './executor.js'
import { TokenCounter }   from './token-counter.js'
import { TaskQueue }      from './task-queue.js'
import { ModelRouter }    from './model-router.js'
import { ApprovalBroker } from './approval.js'
import { taskStore }      from '../storage/task-store.js'
import { sessionStore }   from '../storage/session-store.js'
import { getDirectSystemPrompt }  from './prompts.js'
import { parseActions, parseNarrative } from '../actions/parser.js'
import { actionRegistry } from '../actions/registry.js'
import { summarizeSession, saveSummaryFile, loadSummaryFile, listSummaryFiles } from './summarizer.js'

// ─── Sinais para classificação de intenção ───────────────────────────────────

const SIGNALS = {
  // Verbos de criação (+3 cada)
  actionVerbs: [
    'criar', 'cria', 'desenvolver', 'desenvolva', 'implementar', 'implementa',
    'construir', 'constrói', 'fazer', 'faz', 'gerar', 'gera', 'escrever',
    'escreve', 'montar', 'monta', 'configurar', 'configura', 'estruturar',
  ],
  // Substantivos técnicos (+2 cada)
  techNouns: [
    'landing', 'dashboard', 'crud', 'api', 'rest', 'servidor', 'backend',
    'frontend', 'projeto', 'sistema', 'aplicação', 'app', 'site', 'página',
    'script', 'bot', 'cli', 'módulo', 'biblioteca', 'componente', 'serviço',
  ],
  // Modificadores de escopo (+1 cada)
  scopeWords: [
    'completo', 'completa', 'inteiro', 'inteira', 'funcional', 'com',
    'incluindo', 'além', 'também', 'mais', 'tudo',
  ],
}

const THRESHOLD = 3   // pontuação mínima para considerar tarefa complexa

// ─── Fontes que exigem aprovação antes de executar ──────────────────────────
const APPROVAL_SOURCES = new Set(['rest', 'cron', 'webhook'])

export class Maker {
  /**
   * @param {object} opts
   * @param {ModelRouter} opts.router   — roteador de modelos
   * @param {TaskQueue}   opts.queue    — fila de execução
   * @param {object}      opts.config   — configuração global
   */
  constructor({ router, queue, config }) {
    this.router   = router
    this.queue    = queue
    this.config   = config
    this.counter  = null
    this.approval = new ApprovalBroker()
    this._inputs  = []   // referência a todos os inputs ativos
  }

  /**
   * Registra os inputs ativos para broadcast de aprovações.
   */
  setInputs(inputs) {
    this._inputs = inputs
  }

  async init() {
    await actionRegistry.loadSkills()

    let lmStudioUrl = null
    let hfId = null
    let modelName = 'default'

    if (Array.isArray(this.config.models)) {
      // Estrutura unificada
      const activeKey = this.router._resolveKey('default')
      const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))
      const activeCfg = MODELS_REGISTRY[activeKey] || {}
      
      lmStudioUrl = activeCfg.provider === 'lmstudio' ? activeCfg.baseUrl : null
      hfId = activeCfg.hfId
      modelName = activeCfg.name || activeKey
    } else if (this.config.roles) {
      // Nova estrutura: Papéis e Registro de Modelos
      const activeKey = this.router._resolveKey('default')
      const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))
      const activeCfg = MODELS_REGISTRY[activeKey] || {}
      
      lmStudioUrl = activeCfg.provider === 'lmstudio' ? activeCfg.baseUrl : null
      hfId = activeCfg.hfId
      modelName = activeCfg.name || activeKey
    } else {
      // Retrocompatibilidade legado
      const defaultCfg = this.config.models?.default || {}
      lmStudioUrl = defaultCfg.provider === 'lmstudio' ? defaultCfg.baseUrl : null
      hfId = defaultCfg.hfId
      modelName = defaultCfg.name || 'default'
    }

    this.counter = new TokenCounter({
      lmStudioUrl,
      modelId:     hfId,
      modelName,
    })
    await this.counter.init()
  }

  // ─── Ponto de entrada de cada mensagem ──────────────────────────────────

  async handle(msg, input) {
    const { source, userId, text } = msg
    const sessionKey = `${source}:${userId}`

    console.log(`[maker] <${sessionKey}> ${text.slice(0, 80)}`)

    if (text.trim().startsWith('/')) {
      return this.handleCommand(text.trim(), userId, input, source)
    }

    const isComplex = this.classifyIntent(text)

    if (isComplex) {
      await this.startTask(text, { source, userId, sessionKey }, input)
    } else {
      await this.replyDirect(text, sessionKey, userId, input)
    }
  }

  // ─── Classificação por pontuação de sinais ───────────────────────────────

  classifyIntent(text) {
    const lower = text.toLowerCase()
    let score = 0

    for (const verb of SIGNALS.actionVerbs) {
      if (lower.includes(verb)) { score += 3; break }
    }
    for (const noun of SIGNALS.techNouns) {
      if (lower.includes(noun)) { score += 2; break }
    }
    for (const mod of SIGNALS.scopeWords) {
      if (lower.includes(mod)) score += 1
    }

    if (text.length > 60)  score += 1
    if (text.length > 120) score += 1

    const sentences = text.split(/[.!?]/).filter(s => s.trim().length > 3)
    if (sentences.length >= 2) score += 1

    console.log(`[maker] classifyIntent score=${score} threshold=${THRESHOLD}`)
    return score >= THRESHOLD
  }

  // ─── Tarefa multi-fase (com aprovação para fontes externas) ────────────────

  async startTask(goal, { source, userId, sessionKey }, input) {
    await input.sendTyping(userId)

    const plannerModel = this.router.forPlanning()
    const plannerModelName = plannerModel.modelName ?? plannerModel.model ?? plannerModel.constructor.name ?? 'desconhecido'

    console.log(`[maker] O modelo ${plannerModelName} está planejando para a tarefa: "${goal.slice(0, 60)}"`)
    await input.send(userId, `Modelo: ${plannerModelName} - Planejando as etapas...`)

    // Planner gera as fases
    let phases
    try {
      phases = await planTask(goal, plannerModel)
    } catch (err) {
      await input.send(userId, `Não consegui planejar: ${err.message}`)
      return
    }

    const taskId = `task-${crypto.randomUUID().slice(0, 8)}`
    
    let taskModelName = 'default'
    if (Array.isArray(this.config.models)) {
      taskModelName = this.router._resolveKey('default')
    } else if (this.config.roles) {
      taskModelName = this.router._resolveKey('default')
    } else {
      taskModelName = this.config.models?.default?.name || 'default'
    }

    const task = {
      id:        taskId,
      source,    userId,    goal,
      model:     taskModelName,
      status:    'planned',
      phases,
      createdAt: new Date().toISOString(),
    }

    await taskStore.save(task)

    const planText = phases.map((p, i) => `${i + 1}. ${p.name}`).join('\n')

    // ─── Aprovação para fontes externas (REST, cron, webhook) ─────────
    if (APPROVAL_SOURCES.has(source)) {
      const question =
        `📋 Tarefa recebida via *${source}*:\n` +
        `"${goal.slice(0, 120)}"\n\n` +
        `Plano (${phases.length} fases):\n${planText}`

      // Notifica o canal de origem
      await input.send(userId, `${question}\n\n⏳ Aguardando aprovação...`)

      // Broadcast para terminal + telegram (exclui o REST)
      const humanInputs = this._inputs.filter(i => i.name !== 'rest')
      const approved = await this.approval.request(taskId, question, humanInputs)

      if (!approved) {
        task.status = 'rejected'
        await taskStore.save(task)
        await input.send(userId, `❌ Tarefa ${taskId} rejeitada.`)

        // Notifica os canais humanos também
        for (const hi of humanInputs) {
          hi.send?.('broadcast', `❌ Tarefa ${taskId} rejeitada.`).catch(() => {})
        }
        return
      }

      // Aprovada — notifica todos
      const executorModel = this.router.forExecution()
      const executorModelName = executorModel.modelName ?? executorModel.model ?? executorModel.constructor.name ?? 'desconhecido'
      for (const hi of humanInputs) {
        hi.send?.('broadcast', `✅ Tarefa ${taskId} aprovada — Modelo: ${executorModelName} - Iniciando execução...`).catch(() => {})
      }
      await input.send(userId, `✅ Aprovada — Modelo: ${executorModelName} - Iniciando execução...`)
    } else {
      // Fontes humanas (terminal, telegram) executam direto
      const executorModel = this.router.forExecution()
      const executorModelName = executorModel.modelName ?? executorModel.model ?? executorModel.constructor.name ?? 'desconhecido'
      await input.send(userId, `Plano (${phases.length} fases):\n${planText}\n\nModelo: ${executorModelName} - Iniciando execução...`)
    }

    // ─── Execução ─────────────────────────────────────────────────────
    task.status = 'running'
    await taskStore.save(task)

    // Escolhe o input para feedback: REST não tem como mostrar progresso em tempo real,
    // então usa o primeiro input humano disponível como fallback para notificações
    const feedbackInput = APPROVAL_SOURCES.has(source)
      ? (this._inputs.find(i => i.name === 'telegram') ?? this._inputs.find(i => i.name === 'terminal') ?? input)
      : input

    const execution = this.queue.add(
      () => executeTask(task, this.router, this.counter, feedbackInput),
      goal.slice(0, 40)
    )

    const { queued } = this.queue.status
    if (queued > 0) {
      await feedbackInput.send(userId, `Aguardando fila (posição ${queued})...`)
    }

    execution.catch(err => {
      console.error(`[maker] tarefa ${taskId} falhou:`, err.message)
    })
  }

  // ─── Resposta direta (com loop agêntico para ações shell) ─────────────────

  async replyDirect(text, sessionKey, userId, input) {
    await input.sendTyping(userId)

    const directModel = this.router.forDirect()
    const directModelName = directModel.modelName ?? directModel.model ?? directModel.constructor.name ?? 'desconhecido'
    console.log(`[maker] O modelo ${directModelName} está gerando resposta direta...`)

    let history = sessionStore.get(sessionKey)

    // ─── Cenário 1: resumo automático quando o contexto está cheio ───────────
    if (history.length > 0) {
      const modelContext  = directModel.context || {}
      const globalContext = this.config.context || {}
      const maxTokens     = modelContext.maxTokens    ?? globalContext.maxTokens    ?? 8_192
      const reserveOutput = modelContext.reserveOutput ?? globalContext.reserveOutput ?? 2_048

      const probe = [...history, { role: 'user', content: text }]
      const { fits } = await this.counter.willFit(probe, maxTokens, reserveOutput)

      if (!fits) {
        console.log('[session] contexto cheio — comprimindo via LLM...')
        try {
          const summaryText = await summarizeSession(history, directModel)
          const filepath    = await saveSummaryFile(sessionKey, summaryText)
          console.log(`[session] resumo salvo em: ${filepath}`)

          // Substitui histórico pelo resumo compacto
          sessionStore.clear(sessionKey)
          const summaryMsg = { role: 'user', content: `📋 Resumo da conversa anterior:\n${summaryText}` }
          sessionStore.push(sessionKey, summaryMsg)
          history = [summaryMsg]

          await input.send(userId, `💾 Contexto comprimido automaticamente e salvo em:\n\`${filepath}\``)
        } catch (err) {
          console.warn(`[session] falha ao resumir contexto: ${err.message}`)
        }
      }
    }

    const messages = [...history, { role: 'user', content: text }]

    const MAX_TOOL_LOOPS = 3

    for (let i = 0; i < MAX_TOOL_LOOPS; i++) {
      const { text: reply } = await this.router.forDirect().generate(messages, {
        system: getDirectSystemPrompt(actionRegistry.getActionsSchema()),
      })

      const actions = parseActions(reply)

      if (!actions.length) {
        sessionStore.push(sessionKey, { role: 'user',      content: text  })
        sessionStore.push(sessionKey, { role: 'assistant', content: reply })
        await input.send(userId, reply)
        return
      }

      console.log(`[maker] direct: executando ${actions.length} ação(ões) (loop ${i + 1})`)
      const { narrative, results } = await actionRegistry.run(reply, { taskId: null, userId })

      const feedback = results.map(r =>
        `<result action="${r.action}" status="${r.status}">\n${(r.output || '').slice(0, 3000)}\n</result>`
      ).join('\n')

      messages.push({ role: 'assistant', content: reply })
      messages.push({ role: 'user', content: `Resultado da execução:\n${feedback}\n\nAgora resuma o resultado de forma clara e objetiva para o usuário.` })

      await input.sendTyping(userId)
    }

    const { text: finalReply } = await this.router.forDirect().generate(messages, {
      system: getDirectSystemPrompt(actionRegistry.getActionsSchema()),
    })

    const cleanReply = parseNarrative(finalReply)
    sessionStore.push(sessionKey, { role: 'user',      content: text  })
    sessionStore.push(sessionKey, { role: 'assistant', content: cleanReply })
    await input.send(userId, cleanReply)
  }

  // ─── Comandos especiais ───────────────────────────────────────────────────

  async handleCommand(text, userId, input, source) {
    const sessionKey = `${source}:${userId}`
    const [cmd, ...args] = text.slice(1).split(' ')

    switch (cmd) {
      case 'approve': {
        const [taskId] = args
        if (!taskId) { await input.send(userId, 'uso: /approve <taskId>'); break }
        const handled = this.approval.respond(taskId, true)
        await input.send(userId, handled
          ? `✅ Tarefa ${taskId} aprovada.`
          : `Nenhuma aprovação pendente para ${taskId}`
        )
        break
      }

      case 'reject': {
        const [taskId] = args
        if (!taskId) { await input.send(userId, 'uso: /reject <taskId>'); break }
        const handled = this.approval.respond(taskId, false)
        await input.send(userId, handled
          ? `❌ Tarefa ${taskId} rejeitada.`
          : `Nenhuma aprovação pendente para ${taskId}`
        )
        break
      }

      case 'pending': {
        const list = this.approval.listPending()
        if (!list.length) {
          await input.send(userId, 'Nenhuma aprovação pendente.')
          break
        }
        const txt = list.map(p => `• ${p.id}: ${p.question}`).join('\n')
        await input.send(userId, `Aprovações pendentes:\n${txt}`)
        break
      }

      case 'status': {
        const { running, queued } = this.queue.status
        const parts = []
        if (running) parts.push(`${running} executando`)
        if (queued)  parts.push(`${queued} na fila`)
        const pendingApprovals = this.approval.listPending().length
        if (pendingApprovals) parts.push(`${pendingApprovals} aguardando aprovação`)
        
        let msgStatus = parts.length ? parts.join(', ') : 'Nenhuma tarefa ativa'

        const runningTasks = await taskStore.listByStatus('running')
        if (runningTasks.length > 0) {
          const taskDetails = []
          for (const t of runningTasks) {
            let modelName = 'desconhecido'
            let maxTokens = 8192
            let reserveOutput = 2048

            if (Array.isArray(this.config.models)) {
              const activeKey = this.router._resolveKey('executor')
              const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))
              const activeCfg = MODELS_REGISTRY[activeKey] || {}
              modelName = activeCfg.name || activeKey
              const modelContext = activeCfg.context || {}
              const globalContext = this.config.context || {}
              maxTokens = modelContext.maxTokens ?? globalContext.maxTokens ?? 8192
              reserveOutput = modelContext.reserveOutput ?? globalContext.reserveOutput ?? 2048
            } else if (this.config.roles) {
              const activeKey = this.router._resolveKey('executor')
              const { MODELS_REGISTRY } = await import('../config/models-registry.js').catch(() => ({ MODELS_REGISTRY: {} }))
              const activeCfg = MODELS_REGISTRY[activeKey] || {}
              modelName = activeCfg.name || activeKey
              const modelContext = activeCfg.context || {}
              const globalContext = this.config.context || {}
              maxTokens = modelContext.maxTokens ?? globalContext.maxTokens ?? 8192
              reserveOutput = modelContext.reserveOutput ?? globalContext.reserveOutput ?? 2048
            } else {
              const modelConfig = this.config.models?.executor || this.config.models?.default || {}
              modelName = modelConfig.name || 'desconhecido'
              const modelContext = modelConfig.context || {}
              const globalContext = this.config.context || {}
              maxTokens = modelContext.maxTokens ?? globalContext.maxTokens ?? 8192
              reserveOutput = modelContext.reserveOutput ?? globalContext.reserveOutput ?? 2048
            }

            taskDetails.push(`• "${t.goal.slice(0, 45)}" | Modelo: ${modelName} | Contexto: ${maxTokens.toLocaleString()} tokens (${reserveOutput.toLocaleString()} reserva)`)
          }
          msgStatus += `\n\nTarefas em execução:\n${taskDetails.join('\n')}`
        }

        console.log(`[status] ${msgStatus.replace(/\n/g, ' | ')}`)
        await input.send(userId, msgStatus)
        break
      }

      case 'tasks': {
        const tasks = await taskStore.listByUser(userId)
        if (!tasks.length) { await input.send(userId, 'Nenhuma tarefa encontrada'); break }
        const list = tasks.slice(0, 5)
          .map(t => `[${t.status}] ${t.goal.slice(0, 50)} (${t.id})`)
          .join('\n')
        await input.send(userId, `Últimas tarefas:\n${list}`)
        break
      }

      case 'retry': {
        const [taskId] = args
        if (!taskId) { await input.send(userId, 'uso: /retry <taskId>'); break }
        const task = await taskStore.load(taskId)
        if (!task) { await input.send(userId, `Tarefa não encontrada: ${taskId}`); break }

        task.status = 'running'
        await taskStore.save(task)
        await input.send(userId, `Retomando: ${task.goal}`)

        this.queue.add(
          () => executeTask(task, this.router, this.counter, input),
          `retry:${taskId}`
        ).catch(err => console.error(`[maker] retry ${taskId} falhou:`, err.message))
        break
      }

      case 'queue': {
        const s = this.queue.status
        await input.send(userId,
          `Fila: ${s.running} rodando, ${s.queued} aguardando\n` +
          `Concorrência: ${s.concurrency}\n` +
          `Total: ${s.stats.completed} concluídas, ${s.stats.failed} com erro`
        )
        break
      }

      case 'skills': {
        const [sub] = args
        if (sub === 'reload') {
          await input.send(userId, '🔄 Recarregando skills dinâmicas...')
          await actionRegistry.loadSkills()
          const count = actionRegistry.getActionsSchema().length - 3 // total menos as 3 estáticas
          await input.send(userId, `✅ Skills recarregadas! Total de skills dinâmicas ativas: ${count}`)
          break
        }

        const schemas = actionRegistry.getActionsSchema()
        if (!schemas.length) {
          await input.send(userId, 'Nenhuma skill ou ação registrada.')
          break
        }

        const list = schemas.map(s => {
          const paramsList = Object.keys(s.params).length 
            ? ` (params: ${Object.keys(s.params).join(', ')})`
            : ''
          return `• *${s.name}*: ${s.description}${paramsList}`
        }).join('\n')

        await input.send(userId, `🛠️ *Ações & Skills Disponíveis:*\n\n${list}\n\n💡 Use \`/skills reload\` para atualizar.`)
        break
      }

      case 'resumir': {
        const history = sessionStore.get(sessionKey)
        if (!history.length) {
          await input.send(userId, 'Nenhuma conversa ativa para resumir.')
          break
        }
        await input.send(userId, '⏳ Gerando resumo de contexto...')
        try {
          const directModel = this.router.forDirect()
          const summaryText = await summarizeSession(history, directModel)
          const filepath    = await saveSummaryFile(sessionKey, summaryText)

          // Substitui o histórico pelo resumo
          sessionStore.clear(sessionKey)
          sessionStore.push(sessionKey, { role: 'user', content: `📋 Resumo da conversa anterior:\n${summaryText}` })

          const fname = filepath.split(/[/\\]/).pop()
          await input.send(userId,
            `✅ Contexto resumido e salvo!\n\n` +
            `📄 *Arquivo:* \`${fname}\`\n\n` +
            `📋 *Resumo:*\n${summaryText}\n\n` +
            `💡 Para retomar em outra sessão: \`/carregar ${fname}\``
          )
        } catch (err) {
          await input.send(userId, `❌ Erro ao gerar resumo: ${err.message}`)
        }
        break
      }

      case 'carregar': {
        const [filename] = args
        if (!filename) {
          await input.send(userId, 'Uso: /carregar <nome-do-arquivo.md>\nVeja os disponíveis com /resumos')
          break
        }
        const summaryText = await loadSummaryFile(filename)
        if (!summaryText) {
          await input.send(userId, `❌ Arquivo não encontrado: \`${filename}\`\nVeja os disponíveis com /resumos`)
          break
        }
        // Injeta o resumo como contexto inicial da sessão atual
        sessionStore.clear(sessionKey)
        sessionStore.push(sessionKey, { role: 'user', content: `📋 Contexto carregado de sessão anterior:\n${summaryText}` })
        await input.send(userId,
          `✅ Contexto carregado de \`${filename}\`!\n\n` +
          `Agora posso continuar de onde paramos. O que deseja fazer?`
        )
        break
      }

      case 'resumos': {
        const files = await listSummaryFiles()
        if (!files.length) {
          await input.send(userId, 'Nenhum resumo salvo ainda.\nUse /resumir para criar um.')
          break
        }
        const list = files.slice(0, 10).map((f, i) => `${i + 1}. \`${f}\``).join('\n')
        await input.send(userId,
          `📁 *Resumos disponíveis* (${files.length} no total):\n\n${list}\n\n` +
          `Use \`/carregar <nome-do-arquivo>\` para retomar.`
        )
        break
      }

      default:
        await input.send(userId,
          `Comando desconhecido: /${cmd}\n` +
          `Disponíveis: /status /tasks /queue /retry <id> /approve <id> /reject <id> /pending /skills /resumir /resumos /carregar <arquivo>`
        )
    }
  }

  // ─── Graceful shutdown ────────────────────────────────────────────────────

  async drain(timeoutMs = 10_000) {
    await this.queue.drain(timeoutMs)
  }
}

// ─── Factory: monta o Maker a partir do config ────────────────────────────────

export function createMaker({ router, config }) {
  const queue = new TaskQueue(config.queue ?? { concurrency: 1 })
  return new Maker({ router, queue, config })
}
