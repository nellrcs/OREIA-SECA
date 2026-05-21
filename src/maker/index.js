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
import { DIRECT_SYSTEM }  from './prompts.js'
import { parseActions, parseNarrative } from '../actions/parser.js'
import { actionRegistry } from '../actions/registry.js'

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
    // TokenCounter usa o modelo default para inferir família/tokenizer
    const defaultCfg = this.config.models.default
    this.counter = new TokenCounter({
      lmStudioUrl: defaultCfg.baseUrl,
      modelId:     defaultCfg.hfId,
      modelName:   defaultCfg.name,
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
    await input.send(userId, 'Planejando as etapas...')

    // Planner gera as fases
    let phases
    try {
      phases = await planTask(goal, this.router.forPlanning())
    } catch (err) {
      await input.send(userId, `Não consegui planejar: ${err.message}`)
      return
    }

    const taskId = `task-${crypto.randomUUID().slice(0, 8)}`
    const task = {
      id:        taskId,
      source,    userId,    goal,
      model:     this.config.models.default.name,
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
      for (const hi of humanInputs) {
        hi.send?.('broadcast', `✅ Tarefa ${taskId} aprovada — executando...`).catch(() => {})
      }
      await input.send(userId, `✅ Aprovada — executando...`)
    } else {
      // Fontes humanas (terminal, telegram) executam direto
      await input.send(userId, `Plano (${phases.length} fases):\n${planText}\n\nIniciando...`)
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
      () => executeTask(task, this.router.forExecution(), this.counter, feedbackInput),
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

    const history  = sessionStore.get(sessionKey)
    const messages = [...history, { role: 'user', content: text }]

    const MAX_TOOL_LOOPS = 3

    for (let i = 0; i < MAX_TOOL_LOOPS; i++) {
      const { text: reply } = await this.router.forDirect().generate(messages, {
        system: DIRECT_SYSTEM,
      })

      const actions = parseActions(reply)

      if (!actions.length) {
        sessionStore.push(sessionKey, { role: 'user',      content: text  })
        sessionStore.push(sessionKey, { role: 'assistant', content: reply })
        await input.send(userId, reply)
        return
      }

      console.log(`[maker] direct: executando ${actions.length} ação(ões) (loop ${i + 1})`)
      const { narrative, results } = await actionRegistry.run(reply, { taskId: null })

      const feedback = results.map(r =>
        `<result action="${r.action}" status="${r.status}">\n${(r.output || '').slice(0, 3000)}\n</result>`
      ).join('\n')

      messages.push({ role: 'assistant', content: reply })
      messages.push({ role: 'user', content: `Resultado da execução:\n${feedback}\n\nAgora resuma o resultado de forma clara e objetiva para o usuário.` })

      await input.sendTyping(userId)
    }

    const { text: finalReply } = await this.router.forDirect().generate(messages, {
      system: DIRECT_SYSTEM,
    })

    const cleanReply = parseNarrative(finalReply)
    sessionStore.push(sessionKey, { role: 'user',      content: text  })
    sessionStore.push(sessionKey, { role: 'assistant', content: cleanReply })
    await input.send(userId, cleanReply)
  }

  // ─── Comandos especiais ───────────────────────────────────────────────────

  async handleCommand(text, userId, input, source) {
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
        await input.send(userId, parts.length
          ? parts.join(', ')
          : 'Nenhuma tarefa ativa'
        )
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
          () => executeTask(task, this.router.forExecution(), this.counter, input),
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

      default:
        await input.send(userId,
          `Comando desconhecido: /${cmd}\n` +
          `Disponíveis: /status /tasks /queue /retry <id> /approve <id> /reject <id> /pending`
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
