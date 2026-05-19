// test/integration.js
const c = {
  reset: '\x1b[0m', green: '\x1b[32m', red: '\x1b[31m',
  cyan: '\x1b[36m', bold: '\x1b[1m', gray: '\x1b[90m',
}
let passed = 0, failed = 0

function section(name) { console.log(`\n${c.cyan}${c.bold}══ ${name} ══${c.reset}`) }
async function test(name, fn) {
  try   { await fn(); console.log(`  ${c.green}✓${c.reset} ${name}`); passed++ }
  catch (err) { console.log(`  ${c.red}✗${c.reset} ${name}\n    ${c.red}${err.message}${c.reset}`); failed++ }
}
function assert(cond, msg) { if (!cond) throw new Error(msg ?? 'assertion falhou') }
function assertEqual(a, b, msg) {
  if (a !== b) throw new Error(msg ?? `esperado ${JSON.stringify(b)}, recebeu ${JSON.stringify(a)}`)
}

const sleep = ms => new Promise(r => setTimeout(r, ms))

// ─── Mocks ────────────────────────────────────────────────────────────────────

class MockInput {
  constructor() { this.messages = []; this.typings = 0 }
  async send(_, text) { this.messages.push(text) }
  async sendTyping()  { this.typings++ }
  last()  { return this.messages.at(-1) ?? '' }
  all()   { return this.messages }
}

class MockModel {
  constructor(responses = []) { this.responses = [...responses]; this.calls = 0 }
  async generate() {
    this.calls++
    const r = this.responses.shift() ?? 'resposta padrão'
    return { text: r, usage: { prompt_tokens: 30, completion_tokens: 50 } }
  }
  async reload() {}
  get model() { return 'mock-model' }
  async init() {}
}

// ─── TaskQueue ────────────────────────────────────────────────────────────────

section('TaskQueue')
const { TaskQueue } = await import('../src/maker/task-queue.js')

await test('executa tarefa imediatamente quando fila vazia', async () => {
  const q = new TaskQueue({ concurrency: 1 })
  let ran = false
  await q.add(async () => { ran = true }, 'teste')
  assert(ran)
})

await test('executa em sequência com concurrency=1', async () => {
  const q = new TaskQueue({ concurrency: 1 })
  const order = []
  const t1 = q.add(async () => { await sleep(30); order.push(1) }, 't1')
  const t2 = q.add(async () => { order.push(2) }, 't2')
  await Promise.all([t1, t2])
  assertEqual(order[0], 1, 'tarefa 1 deve rodar primeiro')
  assertEqual(order[1], 2, 'tarefa 2 deve rodar segundo')
})

await test('executa em paralelo com concurrency=2', async () => {
  const q = new TaskQueue({ concurrency: 2 })
  const starts = []
  const t1 = q.add(async () => { starts.push(Date.now()); await sleep(40) }, 't1')
  const t2 = q.add(async () => { starts.push(Date.now()); await sleep(40) }, 't2')
  await Promise.all([t1, t2])
  const gap = Math.abs(starts[1] - starts[0])
  assert(gap < 20, `tarefas devem iniciar juntas, gap=${gap}ms`)
})

await test('status reflete running e queued corretamente', async () => {
  const q = new TaskQueue({ concurrency: 1 })
  let checkedInsde = false
  const t1 = q.add(async () => {
    await sleep(50)
    checkedInsde = true
    assertEqual(q.status.running, 1)
    assertEqual(q.status.queued,  1)
  }, 't1')
  const t2 = q.add(async () => {}, 't2')
  await Promise.all([t1, t2])
  assert(checkedInsde)
  assertEqual(q.status.running, 0)
  assertEqual(q.status.queued,  0)
})

await test('propaga erro da tarefa sem travar a fila', async () => {
  const q = new TaskQueue({ concurrency: 1 })
  let t1Threw = false
  let t2Ran   = false
  await q.add(async () => { throw new Error('erro proposital') }, 't1').catch(() => { t1Threw = true })
  await q.add(async () => { t2Ran = true }, 't2')
  assert(t1Threw, 'deve propagar o erro')
  assert(t2Ran,   'fila deve continuar após erro')
})

await test('acumula stats de completed e failed', async () => {
  const q = new TaskQueue({ concurrency: 1 })
  await q.add(async () => {}, 'ok1')
  await q.add(async () => {}, 'ok2')
  await q.add(async () => { throw new Error('x') }, 'fail').catch(() => {})
  assertEqual(q.status.stats.completed, 2)
  assertEqual(q.status.stats.failed,    1)
})

await test('timeout encerra tarefa lenta', async () => {
  const q = new TaskQueue({ concurrency: 1, timeout: 50 })
  let threw = false
  await q.add(async () => sleep(200), 'lenta').catch(() => { threw = true })
  assert(threw, 'deve lançar por timeout')
})

await test('drain aguarda todas as tarefas ativas', async () => {
  const q = new TaskQueue({ concurrency: 2 })
  let done = 0
  q.add(async () => { await sleep(40); done++ }, 'a')
  q.add(async () => { await sleep(40); done++ }, 'b')
  await q.drain(500)
  assertEqual(done, 2, 'drain deve aguardar ambas')
})

// ─── ModelRouter ──────────────────────────────────────────────────────────────

section('ModelRouter')
const { ModelRouter, createRouter } = await import('../src/maker/model-router.js')

await test('lança se models.default ausente', () => {
  let threw = false
  try { new ModelRouter({}) } catch { threw = true }
  assert(threw)
})

await test('forPlanning cai no default quando planner ausente', () => {
  const def = new MockModel()
  const router = new ModelRouter({ default: def })
  assert(router.forPlanning() === def)
})

await test('forPlanning usa planner quando configurado', () => {
  const def     = new MockModel()
  const planner = new MockModel()
  const router  = new ModelRouter({ default: def, planner })
  assert(router.forPlanning()  === planner, 'planner deve ser usado')
  assert(router.forExecution() === def,     'executor deve cair no default')
})

await test('forExecution usa executor quando configurado', () => {
  const def      = new MockModel()
  const executor = new MockModel()
  const router   = new ModelRouter({ default: def, executor })
  assert(router.forExecution() === executor)
  assert(router.forPlanning()  === def)
})

await test('forDirect usa direct quando configurado', () => {
  const def    = new MockModel()
  const direct = new MockModel()
  const router = new ModelRouter({ default: def, direct })
  assert(router.forDirect() === direct)
})

await test('initAll chama init() em cada modelo único', async () => {
  const inited = []
  const makeM = id => ({
    model: id, calls: 0,
    async init() { inited.push(id) },
    async generate() { return { text: '', usage: { prompt_tokens: 0, completion_tokens: 0 } } }
  })
  const m1 = makeM('m1')
  const m2 = makeM('m2')
  // m1 é usado em dois papéis — init deve ser chamado só uma vez
  const router = new ModelRouter({ default: m1, planner: m1, executor: m2 })
  await router.initAll()
  assertEqual(inited.filter(x => x === 'm1').length, 1, 'm1 init uma vez só')
  assertEqual(inited.filter(x => x === 'm2').length, 1, 'm2 init uma vez')
})

await test('describe retorna papel e nome de cada modelo', () => {
  const def = new MockModel()
  const router = new ModelRouter({ default: def, planner: def })
  const desc = router.describe()
  assert(Array.isArray(desc))
  assert(desc.some(d => d.role === 'default'))
  assert(desc.some(d => d.role === 'planner'))
})

await test('createRouter monta router a partir do config', async () => {
  const cfg = {
    models: {
      default:  { provider: 'lmstudio', name: 'qwen-4b', baseUrl: 'x' },
      executor: { provider: 'lmstudio', name: 'qwen-7b', baseUrl: 'x' },
    }
  }
  const factory = ({ name }) => ({ model: name, async init() {} })
  const router = await createRouter(cfg, factory)
  assert(router.forExecution().model === 'qwen-7b')
  assert(router.forPlanning().model  === 'qwen-4b')
})

// ─── Maker integrado (router + queue) ────────────────────────────────────────

section('Maker integrado')
const { Maker, createMaker } = await import('../src/maker/index.js')

function makeEnv(modelResponses = []) {
  const model  = new MockModel(modelResponses)
  const router = new ModelRouter({ default: model })
  const queue  = new TaskQueue({ concurrency: 1 })
  const config = {
    models:   { default: { name: 'mock', baseUrl: 'http://x', hfId: null } },
    queue:    { concurrency: 1 },
    executor: {},
    inputs:   [],
  }
  const maker = new Maker({ router, queue, config })
  maker.counter = {
    async init() {},
    async count(t) { return Math.ceil(t.length / 4) },
    async willFit(msgs) {
      const used = msgs.reduce((s, m) => s + Math.ceil(m.content.length / 4), 0)
      return { fits: used <= 1148, used, budget: 1148 }
    }
  }
  return { maker, model, router, queue, input: new MockInput() }
}

await test('classifyIntent — pontuação alta para pedido de criação', () => {
  const { maker } = makeEnv()
  assert(maker.classifyIntent('cria uma landing page para meu produto de finanças'))
  assert(maker.classifyIntent('desenvolva um sistema de crud completo com api rest'))
})

await test('classifyIntent — pontuação baixa para pergunta simples', () => {
  const { maker } = makeEnv()
  assert(!maker.classifyIntent('oi'))
  assert(!maker.classifyIntent('o que é uma api?'))
  assert(!maker.classifyIntent('explica o que é rest'))
})

await test('classifyIntent — texto longo sem verbo ainda pode pontuar', () => {
  const { maker } = makeEnv()
  // sistema + aplicação + texto longo = score ≥ 3
  const long = 'quero um sistema de agendamento com aplicação web completa para gerenciar consultas'
  assert(maker.classifyIntent(long))
})

await test('handle — resposta direta para mensagem simples', async () => {
  const { maker, input } = makeEnv(['Olá! Como posso ajudar?'])
  await maker.handle({ source: 'terminal', userId: 'u1', text: 'oi' }, input)
  assert(input.last().includes('Olá'))
})

await test('handle — inicia tarefa para pedido complexo', async () => {
  const { maker, input } = makeEnv([
    JSON.stringify({ phases: [{ id: 'p1', name: 'HTML', instruction: 'crie html' }] }),
    'HTML criado.',
  ])
  await maker.handle(
    { source: 'terminal', userId: 'u2', text: 'cria uma landing page completa' },
    input
  )
  await maker.drain(3000)
  assert(input.all().some(m => m.includes('Planejando')))
  assert(input.all().some(m => m.includes('Tarefa concluída')))
})

await test('fila serializa duas tarefas concorrentes', async () => {
  const order = []
  const q = new TaskQueue({ concurrency: 1 })

  // Simula duas tarefas que registram a ordem de execução
  const t1 = q.add(async () => { await sleep(30); order.push('t1') }, 'tarefa-1')
  const t2 = q.add(async () => { order.push('t2') }, 'tarefa-2')

  await Promise.all([t1, t2])

  assertEqual(order[0], 't1', 'primeira tarefa deve terminar antes da segunda iniciar')
  assertEqual(order[1], 't2')
  assertEqual(q.status.stats.completed, 2)
})

await test('/queue retorna estado da fila', async () => {
  const { maker, input } = makeEnv()
  await maker.handleCommand('/queue', 'u4', input)
  assert(input.last().includes('Fila:'))
  assert(input.last().includes('Concorrência:'))
})

await test('/status usa dados da queue', async () => {
  const { maker, input } = makeEnv()
  await maker.handleCommand('/status', 'u5', input)
  assert(input.last().includes('Nenhuma'))
})

await test('createMaker monta Maker com queue e router corretos', () => {
  const model  = new MockModel()
  const router = new ModelRouter({ default: model })
  const config = {
    models: { default: { name: 'mock', baseUrl: 'http://x', hfId: null } },
    queue:  { concurrency: 2 },
  }
  const maker = createMaker({ router, config })
  assert(maker instanceof Maker)
  assertEqual(maker.queue.concurrency, 2)
})

// ─── Resultado ────────────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)
if (failed > 0) process.exit(1)
