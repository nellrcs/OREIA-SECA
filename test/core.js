// test/core.js
import path from 'path'
import fs   from 'fs/promises'

const c = {
  reset: '\x1b[0m', green: '\x1b[32m', red: '\x1b[31m',
  cyan: '\x1b[36m', bold: '\x1b[1m', gray: '\x1b[90m', yellow: '\x1b[33m',
}

let passed = 0, failed = 0
function section(name) { console.log(`\n${c.cyan}${c.bold}══ ${name} ══${c.reset}`) }
async function test(name, fn) {
  try   { await fn(); console.log(`  ${c.green}✓${c.reset} ${name}`); passed++ }
  catch (err) { console.log(`  ${c.red}✗${c.reset} ${name}\n    ${c.red}${err.message}${c.reset}`); failed++ }
}
function assert(cond, msg) { if (!cond) throw new Error(msg ?? 'assertion falhou') }
function assertEqual(a, b) { if (a !== b) throw new Error(`esperado ${JSON.stringify(b)}, recebeu ${JSON.stringify(a)}`) }

// ─── Mock de canal (input) ───────────────────────────────────────────────────

class MockInput {
  constructor() { this.messages = []; this.typings = 0 }
  async send(userId, text)   { this.messages.push({ userId, text }) }
  async sendTyping(userId)   { this.typings++ }
  last()                     { return this.messages.at(-1)?.text ?? '' }
  all()                      { return this.messages.map(m => m.text) }
}

// ─── Mock de modelo ──────────────────────────────────────────────────────────

class MockModel {
  constructor(responses = []) {
    this.responses = [...responses]
    this.calls     = []
    this.reloads   = 0
  }

  async generate(messages, opts = {}) {
    this.calls.push({ messages, opts })
    const response = this.responses.shift() ?? '(resposta padrão)'
    return {
      text:  response,
      usage: { prompt_tokens: 50, completion_tokens: 80 },
    }
  }

  async reload() { this.reloads++ }
  async init()   {}
  isReady()      { return true }
}

// ─── Mock de TokenCounter ────────────────────────────────────────────────────

class MockCounter {
  async init()                         {}
  async count(text)                    { return Math.ceil(text.length / 4) }
  async willFit(msgs, max = 2048, res = 900) {
    const used = msgs.reduce((s, m) => s + Math.ceil(m.content.length / 4), 0)
    return { fits: used <= (max - res), used, budget: max - res }
  }
}

// ─── TESTES ──────────────────────────────────────────────────────────────────

// 1. Planner
section('Planner')
const { planTask } = await import('../src/maker/planner.js')

await test('gera fases a partir de JSON válido do modelo', async () => {
  const model = new MockModel([
    JSON.stringify({
      phases: [
        { id: 'p1', name: 'Estrutura HTML', instruction: 'Crie o index.html com header e footer' },
        { id: 'p2', name: 'Estilos CSS',    instruction: 'Crie o style.css com reset e layout' },
      ]
    })
  ])
  const phases = await planTask('criar landing page', model)
  assertEqual(phases.length, 2)
  assertEqual(phases[0].name, 'Estrutura HTML')
  assertEqual(phases[0].status, 'pending')
  assert(phases[0].summary === null)
})

await test('tolera JSON com markdown code fence', async () => {
  const model = new MockModel([
    '```json\n{"phases":[{"id":"p1","name":"Setup","instruction":"inicie o projeto"}]}\n```'
  ])
  const phases = await planTask('criar api rest', model)
  assertEqual(phases.length, 1)
  assertEqual(phases[0].name, 'Setup')
})

await test('tolera texto antes e depois do JSON', async () => {
  const model = new MockModel([
    'Claro! Aqui está o plano:\n{"phases":[{"id":"p1","name":"HTML","instruction":"crie o html"}]}\nEspero que ajude!'
  ])
  const phases = await planTask('criar site', model)
  assertEqual(phases.length, 1)
})

await test('faz retry com JSON inválido na primeira tentativa', async () => {
  const model = new MockModel([
    'Aqui está o plano!',   // inválido — sem JSON
    JSON.stringify({ phases: [{ id: 'p1', name: 'Setup', instruction: 'iniciar' }] })
  ])
  const phases = await planTask('criar app', model)
  assertEqual(phases.length, 1)
  assert(model.calls.length >= 2, 'deve ter feito pelo menos 2 chamadas')
})

await test('lança erro após todas as tentativas falharem', async () => {
  const model = new MockModel(['texto inválido', 'outro inválido', 'mais inválido'])
  let threw = false
  try { await planTask('tarefa', model) } catch { threw = true }
  assert(threw, 'deve lançar após esgotar retries')
})

await test('limita a 8 fases mesmo que o modelo retorne mais', async () => {
  const phases = Array.from({ length: 12 }, (_, i) => ({
    id: `p${i+1}`, name: `Fase ${i+1}`, instruction: `instrução ${i+1}`
  }))
  const model = new MockModel([JSON.stringify({ phases })])
  const result = await planTask('projeto grande', model)
  assert(result.length <= 8, `esperado ≤ 8, recebeu ${result.length}`)
})

// 2. Context Builder
section('Context Builder')
const { prepareContext } = await import('../src/maker/context-builder.js')
const counter = new MockCounter()

await test('monta mensagem com goal, fase e histórico', async () => {
  const msgs = await prepareContext({
    goal:  'criar landing page',
    phase: { id: 'p2', name: 'CSS', instruction: 'crie o style.css', status: 'pending', summary: null },
    donePhases: [{ name: 'HTML', status: 'done', summary: 'index.html criado' }],
  }, counter)
  assertEqual(msgs.length, 1)
  assert(msgs[0].content.includes('criar landing page'))
  assert(msgs[0].content.includes('CSS'))
  assert(msgs[0].content.includes('index.html criado'))
})

await test('trunca histórico quando excede budget', async () => {
  // Counter que sempre reporta como não cabendo até o histórico ser removido
  let calls = 0
  const tightCounter = {
    async count(t) { return Math.ceil(t.length / 4) },
    async willFit(msgs) {
      calls++
      const used = msgs.reduce((s, m) => s + Math.ceil(m.content.length / 4), 0)
      // Simula budget de só 50 tokens (muito pequeno)
      return { fits: used <= 50, used, budget: 50 }
    }
  }
  const msgs = await prepareContext({
    goal:  'tarefa',
    phase: { id: 'p5', name: 'Fase 5', instruction: 'instrução curta', status: 'pending', summary: null },
    donePhases: [
      { name: 'F1', status: 'done', summary: 'feito f1' },
      { name: 'F2', status: 'done', summary: 'feito f2' },
      { name: 'F3', status: 'done', summary: 'feito f3' },
      { name: 'F4', status: 'done', summary: 'feito f4' },
    ],
  }, tightCounter)
  assert(msgs.length >= 1, 'deve retornar ao menos uma mensagem')
  assert(calls > 1, 'deve ter tentado múltiplas estratégias')
})

// 3. Executor
section('Executor')
const { executeTask } = await import('../src/maker/executor.js')
await fs.mkdir(path.resolve('workspace'), { recursive: true })
await fs.mkdir(path.resolve('tasks'), { recursive: true })

await test('executa fases em sequência e salva summaries', async () => {
  const input = new MockInput()
  const model = new MockModel([
    // Resposta da fase 1 — cria um arquivo
    `Criando estrutura:\n<action name="file_write"><param name="path">index.html</param><param name="content"><html></html></param></action>\nHTML criado`,
    // Resposta da fase 2 — cria outro arquivo
    `Criando estilos:\n<action name="file_write"><param name="path">style.css</param><param name="content">body{}</param></action>\nCSS criado`,
  ])

  const task = {
    id:     'task-exec-001',
    source: 'terminal', userId: 'local',
    goal:   'criar site simples',
    model:  'lmstudio',
    status: 'running',
    phases: [
      { id: 'p1', name: 'HTML', instruction: 'crie o html', status: 'pending', summary: null },
      { id: 'p2', name: 'CSS',  instruction: 'crie o css',  status: 'pending', summary: null },
    ],
    createdAt: new Date().toISOString(),
  }

  await fs.mkdir(path.resolve('workspace', task.id), { recursive: true })
  await executeTask(task, model, counter, input)

  const saved = await (await import('../src/storage/task-store.js')).taskStore.load(task.id)
  assertEqual(saved.status, 'done')
  assert(saved.phases[0].status === 'done', 'fase 1 deve ser done')
  assert(saved.phases[1].status === 'done', 'fase 2 deve ser done')
  assert(saved.phases[0].summary?.includes('index.html'), `summary inválido: ${saved.phases[0].summary}`)

  const msgs = input.all()
  assert(msgs.some(m => m.includes('✓ HTML')), 'deve confirmar fase HTML')
  assert(msgs.some(m => m.includes('✓ CSS')),  'deve confirmar fase CSS')
  assert(msgs.some(m => m.includes('Tarefa concluída')), 'deve anunciar conclusão')
})

await test('pula fases já concluídas (retomada)', async () => {
  const input = new MockInput()
  const model = new MockModel([
    `<action name="file_write"><param name="path">app.js</param><param name="content">// app</param></action>\nJS criado`,
  ])

  const task = {
    id:     'task-resume-002',
    source: 'terminal', userId: 'local',
    goal:   'criar app',
    model:  'lmstudio',
    status: 'running',
    phases: [
      { id: 'p1', name: 'HTML', instruction: 'html', status: 'done', summary: 'index.html criado' },
      { id: 'p2', name: 'JS',   instruction: 'js',   status: 'pending', summary: null },
    ],
    createdAt: new Date().toISOString(),
  }

  await fs.mkdir(path.resolve('workspace', task.id), { recursive: true })
  await executeTask(task, model, counter, input)

  assertEqual(model.calls.length, 1, 'deve ter chamado o modelo apenas 1x (só a fase pendente)')
  const saved = await (await import('../src/storage/task-store.js')).taskStore.load(task.id)
  assertEqual(saved.phases[1].status, 'done')
})

await test('marca fase como failed se modelo lançar após retries', async () => {
  const input = new MockInput()
  // Modelo que sempre lança erro
  const badModel = {
    async generate() { throw new Error('timeout simulado') },
    async reload()   {},
    async init()     {},
  }

  const task = {
    id:     'task-fail-003',
    source: 'terminal', userId: 'local',
    goal:   'tarefa impossível',
    model:  'lmstudio',
    status: 'running',
    phases: [
      { id: 'p1', name: 'Fase ruim', instruction: 'faça algo', status: 'pending', summary: null },
    ],
    createdAt: new Date().toISOString(),
  }

  await fs.mkdir(path.resolve('workspace', task.id), { recursive: true })
  await executeTask(task, badModel, counter, input)

  const saved = await (await import('../src/storage/task-store.js')).taskStore.load(task.id)
  assertEqual(saved.phases[0].status, 'failed')
  assert(saved.status === 'done_with_errors')
  assert(input.all().some(m => m.includes('erro')))
})

await test('chama reload do modelo após cada fase', async () => {
  const input = new MockInput()
  const model = new MockModel([
    'Fase 1 feita.',
    'Fase 2 feita.',
  ])
  const task = {
    id: 'task-reload-004',
    source: 'terminal', userId: 'local',
    goal: 'testar reload',
    model: 'lmstudio',
    status: 'running',
    phases: [
      { id: 'p1', name: 'F1', instruction: 'fase 1', status: 'pending', summary: null },
      { id: 'p2', name: 'F2', instruction: 'fase 2', status: 'pending', summary: null },
    ],
    createdAt: new Date().toISOString(),
  }
  await fs.mkdir(path.resolve('workspace', task.id), { recursive: true })
  await executeTask(task, model, counter, input)
  assertEqual(model.reloads, 2, `esperado 2 reloads, recebeu ${model.reloads}`)
})

// 4. Maker (orquestrador)
section('Maker — orquestrador')
const { Maker, createMaker } = await import('../src/maker/index.js')
const { ModelRouter }        = await import('../src/maker/model-router.js')
const { TaskQueue }          = await import('../src/maker/task-queue.js')

function makeMaker(responses = []) {
  const model  = new MockModel(responses)
  const router = new ModelRouter({ default: model })
  const queue  = new TaskQueue({ concurrency: 1 })
  const config = { models: { default: { name: 'mock', baseUrl: 'http://x', hfId: null } }, queue: { concurrency: 1 } }
  const maker  = new Maker({ router, queue, config })
  maker.counter = new MockCounter()
  return { maker, model, input: new MockInput() }
}

await test('resposta direta para mensagem simples', async () => {
  const { maker, input } = makeMaker(['Olá! Como posso ajudar?'])
  await maker.handle({ source: 'terminal', userId: 'u1', text: 'oi, tudo bem?' }, input)
  assert(input.messages.length >= 1, 'deve ter enviado resposta')
  assert(input.last().includes('Olá'), `resposta inesperada: ${input.last()}`)
})

await test('classifica mensagem complexa e inicia tarefa', async () => {
  const { maker, input } = makeMaker([
    JSON.stringify({ phases: [{ id: 'p1', name: 'HTML', instruction: 'crie o html' }] }),
    'HTML criado.',
  ])
  await maker.handle(
    { source: 'terminal', userId: 'u2', text: 'cria uma landing page para meu produto' },
    input
  )
  await maker.drain(3000)
  const msgs = input.all()
  assert(msgs.some(m => m.includes('Planejando')), 'deve anunciar planejamento')
  assert(msgs.some(m => m.includes('HTML')), 'deve anunciar a fase')
})

await test('/status retorna contagem de tarefas ativas', async () => {
  const { maker, input } = makeMaker()
  await maker.handleCommand('/status', 'u3', input)
  assert(input.last().includes('Nenhuma'))
})

await test('/tasks retorna histórico do usuário', async () => {
  const { maker, input } = makeMaker()
  const { taskStore } = await import('../src/storage/task-store.js')
  await taskStore.save({
    id: 'task-hist-099', source: 'terminal', userId: 'u4',
    goal: 'tarefa de teste', model: 'mock', status: 'done',
    phases: [], createdAt: new Date().toISOString()
  })
  await maker.handleCommand('/tasks', 'u4', input)
  assert(input.last().includes('tarefa de teste'), `esperado histórico, recebeu: ${input.last()}`)
})

await test('comando desconhecido retorna lista de disponíveis', async () => {
  const { maker, input } = makeMaker()
  await maker.handleCommand('/naoexiste', 'u5', input)
  assert(input.last().includes('Disponíveis'))
})

await test('drain aguarda tarefas ativas', async () => {
  const { maker } = makeMaker()
  let resolved = false
  maker.queue._active.add(
    new Promise(r => setTimeout(() => { resolved = true; r() }, 100))
  )
  maker.queue.running = 1
  await maker.drain(500)
  assert(resolved, 'drain deve ter aguardado a tarefa terminar')
})

// ─── LMStudio (sem servidor — testa só a estrutura) ──────────────────────────
section('LMStudio — estrutura')
const { LMStudioModel } = await import('../src/models/lmstudio.js')

await test('instancia com defaults', () => {
  const m = new LMStudioModel({ model: 'qwen2.5-coder-4b' })
  assertEqual(m.modelName, 'qwen2.5-coder-4b')
  assertEqual(m.baseUrl,   'ws://127.0.0.1:1234')
})

await test('countTokens fallback para heurística quando servidor ausente', async () => {
  const m = new LMStudioModel({ model: 'qwen', baseUrl: 'http://localhost:9999' })
  const n = await m.countTokens('Hello world, this is a test sentence.')
  assert(n > 0 && n < 50, `contagem inválida: ${n}`)
})

await test('reload falha silenciosamente quando servidor ausente', async () => {
  const m = new LMStudioModel({ model: 'qwen', baseUrl: 'http://localhost:9999' })
  await m.reload()   // não deve lançar
  assert(true)
})

await test('init avisa mas não lança quando servidor ausente', async () => {
  const m = new LMStudioModel({ model: 'qwen', baseUrl: 'http://localhost:9999' })
  await m.init()     // não deve lançar
  assert(true)
})

await test('generate lança erro descritivo com status HTTP', async () => {
  const m = new LMStudioModel({ model: 'qwen', baseUrl: 'http://localhost:9999' })
  let threw = false
  try { await m.generate([{ role: 'user', content: 'oi' }]) } catch (err) {
    threw = true
    assert(err.message.length > 0)
  }
  assert(threw)
})

// ─── LocalModel (sem servidor — testa estrutura e resiliência) ─────────────────
section('LocalModel — estrutura')
const { LocalModel } = await import('../src/models/local.js')

await test('instancia com defaults e url correta', () => {
  const m = new LocalModel()
  assertEqual(m.modelName, 'local-model')
  assertEqual(m.baseUrl,   'http://localhost:3000/v1')
})

await test('countTokens usa a heurística da classe base', async () => {
  const m = new LocalModel()
  const n = await m.countTokens('Oi, isso é um teste.')
  assert(n > 0)
})

await test('init falha de forma graciosa e marca offline', async () => {
  const m = new LocalModel({ model: 'qwen', baseUrl: 'http://localhost:9999/v1' })
  await m.init() // não deve lançar
  assert(m.isReady() === false)
})

// ─── Resultado ────────────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)
if (failed > 0) process.exit(1)
