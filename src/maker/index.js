// src/maker/index.js
import crypto             from 'crypto'
import { planTask }       from './planner.js'
import { executeTask }    from './executor.js'
import { TokenCounter }   from './token-counter.js'
import { TaskQueue }      from './task-queue.js'
import { ModelRouter }    from './model-router.js'
import { taskStore }      from '../storage/task-store.js'
import { sessionStore }   from '../storage/session-store.js'
import { DIRECT_SYSTEM }  from './prompts.js'

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

export class Maker {
  /**
   * @param {object} opts
   * @param {ModelRouter} opts.router   — roteador de modelos
   * @param {TaskQueue}   opts.queue    — fila de execução
   * @param {object}      opts.config   — configuração global
   */
  constructor({ router, queue, config }) {
    this.router  = router
    this.queue   = queue
    this.config  = config
    this.counter = null
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
      return this.handleCommand(text.trim(), userId, input)
    }

    const isComplex = this.classifyIntent(text)

    if (isComplex) {
      await this.startTask(text, { source, userId, sessionKey }, input)
    } else {
      await this.replyDirect(text, sessionKey, userId, input)
    }
  }

  // ─── Classificação por pontuação de sinais ───────────────────────────────
  // Substitui a busca por keyword única — agora acumula evidências

  classifyIntent(text) {
    const lower = text.toLowerCase()
    let score = 0

    for (const verb of SIGNALS.actionVerbs) {
      if (lower.includes(verb)) { score += 3; break }   // conta só 1 verbo
    }
    for (const noun of SIGNALS.techNouns) {
      if (lower.includes(noun)) { score += 2; break }   // conta só 1 noun
    }
    for (const mod of SIGNALS.scopeWords) {
      if (lower.includes(mod)) score += 1
    }

    // Texto longo sugere pedido detalhado
    if (text.length > 60)  score += 1
    if (text.length > 120) score += 1

    // Múltiplas frases = mais requisitos
    const sentences = text.split(/[.!?]/).filter(s => s.trim().length > 3)
    if (sentences.length >= 2) score += 1

    console.log(`[maker] classifyIntent score=${score} threshold=${THRESHOLD}`)
    return score >= THRESHOLD
  }

  // ─── Tarefa multi-fase ────────────────────────────────────────────────────

  async startTask(goal, { source, userId, sessionKey }, input) {
    await input.sendTyping(userId)
    await input.send(userId, 'Planejando as etapas...')

    // Planner usa modelo rápido (pode ser diferente do executor)
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
      status:    'running',
      phases,
      createdAt: new Date().toISOString(),
    }

    await taskStore.save(task)

    const planText = phases.map((p, i) => `${i + 1}. ${p.name}`).join('\n')
    await input.send(userId, `Plano (${phases.length} fases):\n${planText}\n\nIniciando...`)

    // Enfileira — garante execução serial quando concurrency = 1
    const execution = this.queue.add(
      () => executeTask(task, this.router.forExecution(), this.counter, input),
      goal.slice(0, 40)
    )

    // Informa posição se foi enfileirada (queue.add é síncrono até o add)
    const { queued } = this.queue.status
    if (queued > 0) {
      await input.send(userId, `Aguardando fila (posição ${queued})...`)
    }

    execution.catch(err => {
      console.error(`[maker] tarefa ${taskId} falhou:`, err.message)
    })
  }

  // ─── Resposta direta ──────────────────────────────────────────────────────

  async replyDirect(text, sessionKey, userId, input) {
    await input.sendTyping(userId)

    const history  = sessionStore.get(sessionKey)
    const messages = [...history, { role: 'user', content: text }]

    const { text: reply } = await this.router.forDirect().generate(messages, {
      system: DIRECT_SYSTEM,
    })

    sessionStore.push(sessionKey, { role: 'user',      content: text  })
    sessionStore.push(sessionKey, { role: 'assistant', content: reply })

    await input.send(userId, reply)
  }

  // ─── Comandos especiais ───────────────────────────────────────────────────

  async handleCommand(text, userId, input) {
    const [cmd, ...args] = text.slice(1).split(' ')

    switch (cmd) {
      case 'status': {
        const { running, queued } = this.queue.status
        const parts = []
        if (running) parts.push(`${running} executando`)
        if (queued)  parts.push(`${queued} na fila`)
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
          `Comando desconhecido: /${cmd}\nDisponíveis: /status /tasks /retry <id> /queue`
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
