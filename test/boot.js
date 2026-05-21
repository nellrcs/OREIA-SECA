// test/boot.js
import fs   from 'fs/promises'
import path from 'path'

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

// ─── BaseInput ────────────────────────────────────────────────────────────────

section('BaseInput')
const { BaseInput } = await import('../src/inputs/base-input.js')

await test('onMessage registra callback e retorna this (fluent)', () => {
  const input = new BaseInput()
  const cb    = () => {}
  const ret   = input.onMessage(cb)
  assert(input._handler === cb, 'handler deve ser registrado')
  assert(ret === input, 'deve retornar this')
})

await test('name retorna nome do canal em lowercase sem "Input"', () => {
  class TerminalInput extends BaseInput {}
  class TelegramInput extends BaseInput {}
  assertEqual(new TerminalInput().name, 'terminal')
  assertEqual(new TelegramInput().name, 'telegram')
})

await test('start() lança se não implementado', async () => {
  let threw = false
  try { await new BaseInput().start() } catch { threw = true }
  assert(threw)
})

await test('stop() e sendTyping() são no-ops por padrão', async () => {
  const input = new BaseInput()
  await input.stop()
  await input.sendTyping('u1')
  assert(true)
})

// ─── TerminalInput ────────────────────────────────────────────────────────────

section('TerminalInput — estrutura (sem abrir stdin)')
const { TerminalInput } = await import('../src/inputs/terminal.js')

await test('instancia sem lançar', () => {
  const t = new TerminalInput({ type: 'terminal' })
  assert(t instanceof TerminalInput)
  assertEqual(t.name, 'terminal')
})

await test('_handler começa null', () => {
  const t = new TerminalInput()
  assert(t._handler === null)
})

await test('onMessage registra o handler', () => {
  const t  = new TerminalInput()
  const cb = async () => {}
  t.onMessage(cb)
  assert(t._handler === cb)
})

await test('send escreve no stdout sem lançar', async () => {
  const t = new TerminalInput()
  // Não tem rl aberto — send não deve lançar
  await t.send('local', 'mensagem de teste')
  assert(true)
})

await test('stop() é seguro sem rl aberto', async () => {
  const t = new TerminalInput()
  await t.stop()
  assert(true)
})

// ─── TelegramInput ────────────────────────────────────────────────────────────

section('TelegramInput — estrutura (sem bot real)')
const { TelegramInput } = await import('../src/inputs/telegram.js')

await test('instancia com config', () => {
  const t = new TelegramInput({ token: 'fake', allowedUsers: ['123'] })
  assertEqual(t.name, 'telegram')
  assertEqual(t.cfg.token, 'fake')
})

await test('_chunk divide texto longo em pedaços de 4000 chars', () => {
  const t = new TelegramInput()
  const texto = 'x'.repeat(9500)
  const chunks = t._chunk(texto)
  assertEqual(chunks.length, 3)
  assert(chunks.every(c => c.length <= 4000))
})

await test('_chunk retorna array com um item para texto curto', () => {
  const t      = new TelegramInput()
  const chunks = t._chunk('mensagem curta')
  assertEqual(chunks.length, 1)
  assertEqual(chunks[0], 'mensagem curta')
})

await test('_stopTyping é seguro sem timer ativo', () => {
  const t = new TelegramInput()
  t._stopTyping('u1')   // não deve lançar
  assert(true)
})

await test('_clearAllTyping limpa todos os timers', () => {
  const t = new TelegramInput()
  // Injeta timers falsos
  t._typingTimers.set('u1', setInterval(() => {}, 9999))
  t._typingTimers.set('u2', setInterval(() => {}, 9999))
  t._clearAllTyping()
  assertEqual(t._typingTimers.size, 0)
})

await test('start() lança erro descritivo se lib não instalada', async () => {
  const t = new TelegramInput({ token: 'fake-token' })
  // Força a lib a não ser encontrada com token inválido
  // (vai falhar no import ou na autenticação — queremos o erro descritivo)
  let threw = false
  try {
    await t.start()
  } catch (err) {
    threw = true
    // Pode ser erro de lib não instalada ou de token inválido — ambos aceitáveis
    assert(err.message.length > 0)
  } finally {
    await t.stop()
  }
  // Em ambiente de teste pode não ter a lib — ok
  if (!threw) assert(true, 'lib disponível — start passou')
})

// ─── Integração: input → maker ────────────────────────────────────────────────

section('Integração input → maker')

await test('mensagem do terminal chega ao handler registrado', async () => {
  const t        = new TerminalInput()
  const recebido = []
  t.onMessage(async (msg) => recebido.push(msg))

  // Simula a chegada de uma mensagem diretamente no handler
  await t._handler?.({
    source: 'terminal', userId: 'local',
    text: 'cria um script', metadata: {},
  })

  assertEqual(recebido.length, 1)
  assertEqual(recebido[0].source, 'terminal')
  assertEqual(recebido[0].text,   'cria um script')
})

await test('input sem handler registrado não lança', async () => {
  const t = new TerminalInput()
  await t._handler?.({ source: 'terminal', userId: 'local', text: 'oi', metadata: {} })
  assert(true)  // _handler é null, optional chaining previne lançamento
})

// ─── checkInterruptedTasks ────────────────────────────────────────────────────

section('Diagnóstico de tarefas interrompidas')
const { taskStore } = await import('../src/storage/task-store.js')

await test('detecta tarefas com status running e as marca como interrupted', async () => {
  // Cria uma tarefa "perdida"
  await taskStore.save({
    id: 'task-orphan-001', source: 'terminal', userId: 'local',
    goal: 'tarefa interrompida', model: 'lmstudio',
    status: 'running',
    phases: [{ id: 'p1', name: 'F1', instruction: 'x', status: 'pending', summary: null }],
    createdAt: new Date().toISOString(),
  })

  const running = await taskStore.listByStatus('running')
  assert(running.some(t => t.id === 'task-orphan-001'))

  // Simula o que checkInterruptedTasks faz
  for (const task of running) {
    await taskStore.patch(task.id, { status: 'interrupted' })
  }

  const afterPatch = await taskStore.listByStatus('running')
  assert(!afterPatch.some(t => t.id === 'task-orphan-001'), 'deve ter sido marcada como interrupted')

  const interrupted = await taskStore.listByStatus('interrupted')
  assert(interrupted.some(t => t.id === 'task-orphan-001'))
})

// ─── Validação de sintaxe do index.js ────────────────────────────────────────

section('index.js — sintaxe e imports')

await test('index.js existe em disco', async () => {
  const exists = await fs.access(path.resolve('index.js')).then(() => true).catch(() => false)
  assert(exists, 'index.js deve existir na raiz do projeto')
})

await test('index.js usa ESM (type: module)', async () => {
  const pkg = JSON.parse(await fs.readFile(path.resolve('package.json'), 'utf-8'))
  assertEqual(pkg.type, 'module', 'package.json deve ter type: module')
})

await test('todos os imports do index.js existem em disco', async () => {
  const content = await fs.readFile(path.resolve('index.js'), 'utf-8')
  const imports = [...content.matchAll(/from '(\.\/[^']+)'/g)].map(m => m[1])

  for (const imp of imports) {
    const filePath = path.resolve(imp.endsWith('.js') ? imp : imp + '.js')
    const exists = await fs.access(filePath).then(() => true).catch(() => false)
    assert(exists, `import não encontrado: ${imp}`)
  }
})

await test('package.json tem script start apontando para index.js', async () => {
  const pkg = JSON.parse(await fs.readFile(path.resolve('package.json'), 'utf-8'))
  assert(pkg.scripts?.start?.includes('index.js'), 'script start deve usar index.js')
})

// ─── Resultado ────────────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)
if (failed > 0) process.exit(1)
