// test/approval-rest.js
import http from 'http'
import { ApprovalBroker } from '../src/maker/approval.js'
import { RestInput } from '../src/inputs/rest.js'

const c = {
  reset: '\x1b[0m', green: '\x1b[32m', red: '\x1b[31m',
  cyan: '\x1b[36m', bold: '\x1b[1m', gray: '\x1b[90m',
}
let passed = 0, failed = 0

function section(name) { console.log(`\n${c.cyan}${c.bold}══ ${name} ══${c.reset}`) }
async function test(name, fn) {
  try   { await fn(); console.log(`  ${c.green}✓${c.reset} ${name}`); passed++ }
  catch (err) { console.log(`  ${c.red}✗${c.reset} ${name}\n    ${c.red}${err.stack}${c.reset}`); failed++ }
}
function assert(cond, msg) { if (!cond) throw new Error(msg ?? 'assertion falhou') }
function assertEqual(a, b, msg) {
  if (a !== b) throw new Error(msg ?? `esperado ${JSON.stringify(b)}, recebeu ${JSON.stringify(a)}`)
}

const sleep = ms => new Promise(r => setTimeout(r, ms))

// ─── Mocks para testes de integração de Aprovação ────────────────────────────

class MockInput {
  constructor(name = 'mock') {
    this.name = name
    this.messages = []
    this.typings = 0
    this._handler = null
  }
  onMessage(handler) { this._handler = handler }
  async start() {}
  async stop() {}
  async send(userId, text) { this.messages.push({ userId, text }) }
  async sendTyping(userId) { this.typings++ }
  last() { return this.messages.at(-1)?.text ?? '' }
  all() { return this.messages.map(m => m.text) }
}

class MockModel {
  constructor(responses = []) {
    this.responses = [...responses]
    this.calls     = []
  }
  async generate(messages, opts = {}) {
    this.calls.push({ messages, opts })
    const response = this.responses.shift() ?? '(resposta padrão)'
    return {
      text:  response,
      usage: { prompt_tokens: 50, completion_tokens: 80 },
    }
  }
  async reload() {}
  async init()   {}
  isReady()      { return true }
}

class MockCounter {
  async init() {}
  async count(text) { return Math.ceil(text.length / 4) }
  async willFit(msgs, max = 2048, res = 900) {
    const used = msgs.reduce((s, m) => s + Math.ceil(m.content.length / 4), 0)
    return { fits: used <= (max - res), used, budget: max - res }
  }
}

// ─── Helpers para Requisições HTTP ──────────────────────────────────────────

function httpRequest(method, url, headers = {}, body = null) {
  return new Promise((resolve, reject) => {
    const parsedUrl = new URL(url)
    const options = {
      method,
      hostname: parsedUrl.hostname,
      port: parsedUrl.port,
      path: parsedUrl.pathname + parsedUrl.search,
      headers: {
        'Content-Type': 'application/json',
        ...headers,
      }
    }

    let hasResponse = false

    const req = http.request(options, res => {
      hasResponse = true
      let data = ''
      res.on('data', chunk => data += chunk)
      res.on('end', () => {
        resolve({
          statusCode: res.statusCode,
          headers: res.headers,
          body: data,
          json: () => JSON.parse(data)
        })
      })
    })

    req.on('error', err => {
      if (hasResponse) return
      reject(err)
    })
    if (body) {
      req.write(typeof body === 'string' ? body : JSON.stringify(body))
    }
    req.end()
  })
}

// ─── Testes do ApprovalBroker ────────────────────────────────────────────────

section('ApprovalBroker')

await test('deve registrar pedido pendente e listar', async () => {
  const broker = new ApprovalBroker()
  const mockInput = {
    send: async (userId, msg) => {}
  }

  const promise = broker.request('task-1', 'Testar?', [mockInput], 100)
  assert(broker.isPending('task-1'), 'deve estar pendente')

  const list = broker.listPending()
  assertEqual(list.length, 1)
  assertEqual(list[0].id, 'task-1')

  // deixa expirar
  const result = await promise
  assert(!result, 'deve expirar como rejeitado')
  assert(!broker.isPending('task-1'), 'não deve estar mais pendente')
})

await test('deve resolver true quando aprovado', async () => {
  const broker = new ApprovalBroker()
  const mockInput = {
    send: async (userId, msg) => {}
  }

  const promise = broker.request('task-2', 'Aprovar?', [mockInput], 5000)
  
  const ok = broker.respond('task-2', true)
  assert(ok, 'deve processar resposta com sucesso')

  const result = await promise
  assert(result, 'deve resolver como true')
  assert(!broker.isPending('task-2'), 'não deve estar mais pendente')
})

await test('deve resolver false quando rejeitado', async () => {
  const broker = new ApprovalBroker()
  const mockInput = {
    send: async (userId, msg) => {}
  }

  const promise = broker.request('task-3', 'Rejeitar?', [mockInput], 5000)
  
  const ok = broker.respond('task-3', false)
  assert(ok, 'deve processar resposta com sucesso')

  const result = await promise
  assert(!result, 'deve resolver como false')
})

// ─── Testes do RestInput ─────────────────────────────────────────────────────

section('RestInput')

await test('deve iniciar HTTP server e responder /api/status e /api/log', async () => {
  const rest = new RestInput({ port: 3999 })
  await rest.start()

  try {
    const statusRes = await httpRequest('GET', 'http://localhost:3999/api/status')
    assertEqual(statusRes.statusCode, 200)
    assertEqual(statusRes.json().online, true)

    const logRes = await httpRequest('GET', 'http://localhost:3999/api/log')
    assertEqual(logRes.statusCode, 200)
    assert(Array.isArray(logRes.json().log))
  } finally {
    await rest.stop()
  }
})

await test('deve rejeitar POST /api/message sem api key autorizada', async () => {
  const rest = new RestInput({ port: 3999, apiKey: 'chave-secreta' })
  await rest.start()

  try {
    const resNoAuth = await httpRequest('POST', 'http://localhost:3999/api/message', {}, null)
    assertEqual(resNoAuth.statusCode, 401)

    const resWrongAuth = await httpRequest(
      'POST',
      'http://localhost:3999/api/message',
      { 'Authorization': 'Bearer errada' },
      null
    )
    assertEqual(resWrongAuth.statusCode, 401)

    const resCorrectAuth = await httpRequest(
      'POST',
      'http://localhost:3999/api/message',
      { 'Authorization': 'Bearer chave-secreta' },
      { text: 'oi' }
    )
    assertEqual(resCorrectAuth.statusCode, 202)
  } finally {
    await rest.stop()
  }
})

await test('deve processar POST /api/message e disparar handler', async () => {
  const rest = new RestInput({ port: 3999 })
  let handlerCalled = false
  let receivedMsg = null

  rest.onMessage(async (msg) => {
    handlerCalled = true
    receivedMsg = msg
  })

  await rest.start()

  try {
    const res = await httpRequest('POST', 'http://localhost:3999/api/message', {}, { text: 'Tarefa automatizada' })
    assertEqual(res.statusCode, 202)
    assertEqual(res.json().status, 'accepted')

    // Espera disparo em background
    await sleep(50)
    assert(handlerCalled, 'deve disparar handler registrado')
    assertEqual(receivedMsg.source, 'rest')
    assertEqual(receivedMsg.text, 'Tarefa automatizada')
  } finally {
    await rest.stop()
  }
})

await test('deve processar fluxo completo de aprovação cross-canal para REST input', async () => {
  const { Maker }       = await import('../src/maker/index.js')
  const { ModelRouter } = await import('../src/maker/model-router.js')
  const { TaskQueue }   = await import('../src/maker/task-queue.js')
  const { taskStore }   = await import('../src/storage/task-store.js')

  // 1. Instanciar mocks e inputs
  const terminalInput = new MockInput('terminal')
  const telegramInput = new MockInput('telegram')
  const restInput = new RestInput({ port: 3999 })

  // 2. Configurar o Maker com MockModel (planner retornando fases válidas)
  const plannerResponse = JSON.stringify({
    phases: [
      { id: 'p1', name: 'HTML', instruction: 'crie o html' }
    ]
  })
  // Resposta para execução
  const execResponse = `HTML concluído:\n<action name="file_write"><param name="path">index.html</param><param name="content"><html></html></param></action>`
  
  const model = new MockModel([plannerResponse, execResponse])
  const router = new ModelRouter({ default: model })
  const queue = new TaskQueue({ concurrency: 1 })
  const config = {
    models: { default: { name: 'mock', baseUrl: 'http://x', hfId: null } },
    queue: { concurrency: 1 }
  }
  
  const maker = new Maker({ router, queue, config })
  maker.counter = new MockCounter()
  maker.setInputs([restInput, terminalInput, telegramInput])

  // Registrar handlers nos inputs
  restInput.onMessage(async (msg) => {
    await maker.handle(msg, restInput)
  })
  terminalInput.onMessage(async (msg) => {
    await maker.handle(msg, terminalInput)
  })
  telegramInput.onMessage(async (msg) => {
    await maker.handle(msg, telegramInput)
  })

  await restInput.start()

  try {
    // 3. Enviar mensagem para o REST input
    // "criar landing" pontua bem no classifyIntent (> 3)
    const res = await httpRequest(
      'POST',
      'http://localhost:3999/api/message',
      {},
      { text: 'criar landing page para meu produto' }
    )

    assertEqual(res.statusCode, 202)
    
    // Aguarda o processamento em background (planejamento)
    await sleep(100)

    // 4. Verificar se a tarefa está pendente no ApprovalBroker e se o broadcast foi enviado para Terminal + Telegram
    const pendingList = maker.approval.listPending()
    assertEqual(pendingList.length, 1, 'Deve haver 1 tarefa pendente de aprovação')
    
    const taskId = pendingList[0].id
    
    // Verificar se mensagens de aprovação foram enviadas para Terminal + Telegram
    assert(terminalInput.all().some(m => m.includes('/approve') && m.includes(taskId)), 'Terminal deve receber pedido de aprovação')
    assert(telegramInput.all().some(m => m.includes('/approve') && m.includes(taskId)), 'Telegram deve receber pedido de aprovação')

    await telegramInput._handler({
      source: 'telegram',
      userId: 'user_tele',
      text: `/approve ${taskId}`
    })

    // Aguarda o agendamento e processamento da execução
    await sleep(50)
    await maker.drain(2000)

    // 6. Verificar se a tarefa foi aprovada e executada com sucesso
    const task = await taskStore.load(taskId)
    assertEqual(task.status, 'done', 'Tarefa deve terminar com status done após aprovação')

    // Verificar se notificações de aprovação/execução foram enviadas
    assert(telegramInput.all().some(m => m.includes('aprovada')), 'Telegram deve notificar aprovação')
    assert(terminalInput.all().some(m => m.includes('aprovada')), 'Terminal deve notificar aprovação')

  } finally {
    await restInput.stop()
  }
})

await test('deve processar fluxo completo de rejeição cross-canal para REST input', async () => {
  const { Maker }       = await import('../src/maker/index.js')
  const { ModelRouter } = await import('../src/maker/model-router.js')
  const { TaskQueue }   = await import('../src/maker/task-queue.js')
  const { taskStore }   = await import('../src/storage/task-store.js')

  const terminalInput = new MockInput('terminal')
  const telegramInput = new MockInput('telegram')
  const restInput = new RestInput({ port: 3999 })

  const plannerResponse = JSON.stringify({
    phases: [
      { id: 'p1', name: 'HTML', instruction: 'crie o html' }
    ]
  })
  
  const model = new MockModel([plannerResponse])
  const router = new ModelRouter({ default: model })
  const queue = new TaskQueue({ concurrency: 1 })
  const config = {
    models: { default: { name: 'mock', baseUrl: 'http://x', hfId: null } },
    queue: { concurrency: 1 }
  }
  
  const maker = new Maker({ router, queue, config })
  maker.counter = new MockCounter()
  maker.setInputs([restInput, terminalInput, telegramInput])

  restInput.onMessage(async (msg) => { await maker.handle(msg, restInput) })
  terminalInput.onMessage(async (msg) => { await maker.handle(msg, terminalInput) })
  telegramInput.onMessage(async (msg) => { await maker.handle(msg, telegramInput) })

  await restInput.start()

  try {
    const res = await httpRequest(
      'POST',
      'http://localhost:3999/api/message',
      {},
      { text: 'criar dashboard para admin' }
    )
    assertEqual(res.statusCode, 202)
    
    await sleep(100)

    const pendingList = maker.approval.listPending()
    assertEqual(pendingList.length, 1)
    const taskId = pendingList[0].id

    // Simular rejeição (/reject <taskId>) vinda do Terminal
    await terminalInput._handler({
      source: 'terminal',
      userId: 'user_term',
      text: `/reject ${taskId}`
    })

    await sleep(100)

    // Verificar status
    const task = await taskStore.load(taskId)
    assertEqual(task.status, 'rejected', 'Tarefa deve ser marcada como rejected')

    // Verificar notificações de rejeição
    assert(terminalInput.all().some(m => m.includes('rejeitada')), 'Terminal deve notificar rejeição')
    assert(telegramInput.all().some(m => m.includes('rejeitada')), 'Telegram deve notificar rejeição')
  } finally {
    await restInput.stop()
  }
})

// ─── Resultado ────────────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)
if (failed > 0) process.exit(1)
