// index.js — OREIASECA
import 'dotenv/config'
import { config }        from './src/config.js'
import { LMStudioModel } from './src/models/lmstudio.js'
import { createRouter }  from './src/maker/model-router.js'
import { createMaker }   from './src/maker/index.js'
import { sessionStore }  from './src/storage/session-store.js'
import { taskStore }     from './src/storage/task-store.js'
import { TerminalInput } from './src/inputs/terminal.js'
import { TelegramInput } from './src/inputs/telegram.js'
import { RestInput }     from './src/inputs/rest.js'

// ─── Banner ──────────────────────────────────────────────────────────────────

function banner() {
  console.log(`
\x1b[36m╔═══════════════════════════════════════════╗
║  O.R.E.I.A.S.E.C.A                       ║
║  Orquestrador de Recursos e Experimentos  ║
║  em IA para Sistemas de Execução e        ║
║  Controle Automatizado          v0.1.0    ║
╚═══════════════════════════════════════════╝\x1b[0m
`)
}

// ─── Factory de modelos ───────────────────────────────────────────────────────

async function modelFactory(cfg) {
  switch (cfg.provider) {
    case 'lmstudio':
      return new LMStudioModel({ model: cfg.name, baseUrl: cfg.baseUrl })

    case 'gemini': {
      const { GeminiModel } = await import('./src/models/gemini.js')
      return new GeminiModel({ model: cfg.name, apiKey: cfg.apiKey })
    }

    case 'openrouter': {
      const { OpenRouterModel } = await import('./src/models/openrouter.js')
      return new OpenRouterModel({ model: cfg.name, apiKey: cfg.apiKey })
    }

    case 'local': {
      const { LocalModel } = await import('./src/models/local.js')
      return new LocalModel({ model: cfg.name, baseUrl: cfg.baseUrl, apiKey: cfg.apiKey })
    }

    default:
      throw new Error(
        `Provider desconhecido: "${cfg.provider}"\n` +
        `Valores aceitos: lmstudio, gemini, openrouter, local`
      )
  }
}

// ─── Factory de inputs ────────────────────────────────────────────────────────

function inputFactory(cfg) {
  switch (cfg.type) {
    case 'terminal': return new TerminalInput(cfg)
    case 'telegram': return new TelegramInput(cfg)
    case 'rest':     return new RestInput(cfg)
    default:
      throw new Error(`Input desconhecido: "${cfg.type}"`)
  }
}

// ─── Inicializa e conecta inputs ao Maker ────────────────────────────────────

async function startInputs(maker, inputConfigs) {
  const active = []

  for (const cfg of inputConfigs) {
    if (!cfg.enabled) {
      console.log(`[boot] input "${cfg.type}" desativado — pulando`)
      continue
    }

    let input
    try {
      input = inputFactory(cfg)
    } catch (err) {
      console.warn(`[boot] input "${cfg.type}" ignorado: ${err.message}`)
      continue
    }

    // Conecta cada mensagem ao Maker
    input.onMessage(async (msg) => {
      try {
        await maker.handle(msg, input)
      } catch (err) {
        console.error(`[${input.name}] erro não tratado:`, err)
        await input.send(msg.userId, `Erro interno: ${err.message}`).catch(() => {})
      }
    })

    try {
      await input.start()
      active.push(input)
      console.log(`[boot] input "${input.name}" iniciado`)
    } catch (err) {
      console.warn(`[boot] falha ao iniciar "${cfg.type}": ${err.message}`)
    }
  }

  return active
}

// ─── Graceful shutdown ────────────────────────────────────────────────────────

function setupShutdown(inputs, maker) {
  let shuttingDown = false

  const shutdown = async (signal) => {
    if (shuttingDown) return
    shuttingDown = true

    console.log(`\n[shutdown] ${signal} recebido — encerrando...`)

    // Para de receber novas mensagens
    await Promise.allSettled(inputs.map(i => i.stop()))

    // Aguarda tarefas em andamento (max 15s)
    await maker.drain(15_000)

    // Persiste sessões em disco
    await sessionStore.saveAll()
    console.log('[shutdown] sessões salvas')

    console.log('[shutdown] encerrado')
    process.exit(0)
  }

  process.on('SIGINT',  () => shutdown('SIGINT'))
  process.on('SIGTERM', () => shutdown('SIGTERM'))

  process.on('uncaughtException', (err) => {
    console.error('[fatal] exceção não capturada:', err)
    // Só encerra em erros realmente fatais
    if (err.code === 'ERR_SOCKET_ALREADY_CLOSED' || err.code === 'EPIPE') return
    process.exit(1)
  })

  process.on('unhandledRejection', (reason) => {
    // NÃO encerra o processo — async handlers em EventEmitters geram rejections
    // que são recuperáveis (ex: timeout do modelo, erro de rede do Telegram)
    console.error('[warn] promise rejeitada não tratada:', reason)
  })
}

// ─── Diagnóstico de tarefas interrompidas ────────────────────────────────────

async function checkInterruptedTasks() {
  const running = await taskStore.listByStatus('running')
  if (!running.length) return

  console.warn(`[boot] ${running.length} tarefa(s) interrompida(s) encontrada(s):`)
  for (const task of running) {
    const pending = task.phases.filter(p => p.status === 'pending').length
    console.warn(`  • ${task.id} — "${task.goal.slice(0, 50)}" (${pending} fase(s) pendente(s))`)
    console.warn(`    Use /retry ${task.id} para retomar`)
    // Marca como interrompida para não ficar como "running" indefinidamente
    await taskStore.patch(task.id, { status: 'interrupted' })
  }
}

// ─── Boot ────────────────────────────────────────────────────────────────────

async function main() {
  banner()

  // 1. Restaura sessões do disco
  await sessionStore.loadAll()

  // 2. Avisa sobre tarefas interrompidas (ex: processo morreu no meio)
  await checkInterruptedTasks()

  // 3. Cria o roteador de modelos
  console.log('[boot] inicializando modelos...')
  let router
  try {
    router = await createRouter(config, modelFactory)
  } catch (err) {
    console.error('[boot] falha ao criar router de modelos:', err.message)
    process.exit(1)
  }

  // 4. Cria o Maker (inclui TaskQueue)
  const maker = createMaker({ router, config })
  await maker.init()
  console.log('[boot] maker pronto')

  // 5. Inicia os canais de entrada
  const inputs = await startInputs(maker, config.inputs)

  if (!inputs.length) {
    console.error('[boot] nenhum input ativo — configure ao menos um canal em src/config.js')
    process.exit(1)
  }

  // 6. Registra inputs no Maker para broadcast de aprovações
  maker.setInputs(inputs)

  // 6. Configura shutdown graceful
  setupShutdown(inputs, maker)

  const inputNames = inputs.map(i => i.name).join(', ')
  console.log(`\n[boot] OREIASECA rodando — canais: ${inputNames}`)
}

main().catch(err => {
  console.error('[boot] falha na inicialização:', err)
  process.exit(1)
})
