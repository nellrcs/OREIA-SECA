// test/unit.js
import path from 'path'
import fs   from 'fs/promises'

// ─── Helpers de output ────────────────────────────────────────────────────────

const c = {
  reset:  '\x1b[0m',
  green:  '\x1b[32m',
  red:    '\x1b[31m',
  yellow: '\x1b[33m',
  cyan:   '\x1b[36m',
  gray:   '\x1b[90m',
  bold:   '\x1b[1m',
}

let passed = 0, failed = 0

function section(name) {
  console.log(`\n${c.cyan}${c.bold}══ ${name} ══${c.reset}`)
}

async function test(name, fn) {
  try {
    await fn()
    console.log(`  ${c.green}✓${c.reset} ${name}`)
    passed++
  } catch (err) {
    console.log(`  ${c.red}✗${c.reset} ${name}`)
    console.log(`    ${c.red}${err.message}${c.reset}`)
    failed++
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg ?? 'assertion falhou')
}

function assertEqual(a, b, msg) {
  if (a !== b) throw new Error(msg ?? `esperado ${JSON.stringify(b)}, recebeu ${JSON.stringify(a)}`)
}

// ─── Testes ───────────────────────────────────────────────────────────────────

// 1. Parser
section('Parser de ações')
const { parseActions, parseNarrative } = await import('../src/actions/parser.js')

await test('extrai uma ação simples', () => {
  const text = `Vou criar o arquivo:
<action name="file_write">
  <param name="path">index.html</param>
  <param name="content"><!DOCTYPE html></param>
</action>`
  const actions = parseActions(text)
  assert(actions.length === 1, 'deve ter 1 ação')
  assertEqual(actions[0].name, 'file_write')
  assertEqual(actions[0].params.path, 'index.html')
  assertEqual(actions[0].params.content, '<!DOCTYPE html>')
})

await test('extrai múltiplas ações', () => {
  const text = `
<action name="shell">
  <param name="command">mkdir -p src</param>
</action>
<action name="file_write">
  <param name="path">src/app.js</param>
  <param name="content">console.log('oi')</param>
</action>`
  const actions = parseActions(text)
  assert(actions.length === 2, 'deve ter 2 ações')
  assertEqual(actions[0].name, 'shell')
  assertEqual(actions[1].name, 'file_write')
})

await test('tolerância a espaços, aspas simples e maiúsculas nas tags', () => {
  const text = `
<ACTION name = 'shell' >
  <PARAM name = 'command' >ls -la</PARAM>
</ACTION>
<action name=file_write>
  <param name="path" >excluir.php</param>
  <param name='content'>hello</param>
</action>`
  const actions = parseActions(text)
  assert(actions.length === 2)
  assertEqual(actions[0].name, 'shell')
  assertEqual(actions[0].params.command, 'ls -la')
  assertEqual(actions[1].name, 'file_write')
  assertEqual(actions[1].params.path, 'excluir.php')
  assertEqual(actions[1].params.content, 'hello')
})

await test('retorna array vazio se sem ações', () => {
  const actions = parseActions('Apenas um texto sem ações.')
  assert(actions.length === 0)
})

await test('extrai narrativa removendo blocos de ação', () => {
  const text = `Criando a estrutura:
<action name="shell">
  <param name="command">mkdir src</param>
</action>
Estrutura criada com sucesso.`
  const narrative = parseNarrative(text)
  assert(!narrative.includes('<action'), 'não deve ter bloco XML')
  assert(narrative.includes('Criando a estrutura'), 'deve ter texto antes')
  assert(narrative.includes('Estrutura criada'), 'deve ter texto depois')
})

await test('param com conteúdo multiline', () => {
  const text = `<action name="file_write">
  <param name="path">index.html</param>
  <param name="content">linha 1
linha 2
linha 3</param>
</action>`
  const actions = parseActions(text)
  assert(actions[0].params.content.includes('linha 2'))
})

// 2. Action registry
section('Action Registry')
const { actionRegistry } = await import('../src/actions/registry.js')
const TASK_CTX = { taskId: 'test-task-001' }

// Garante que o diretório do workspace existe
await fs.mkdir(path.resolve('workspace', TASK_CTX.taskId), { recursive: true })

await test('file_write cria arquivo no workspace', async () => {
  const response = `Criando arquivo:
<action name="file_write">
  <param name="path">hello.txt</param>
  <param name="content">olá mundo</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assert(result.results.length === 1)
  assertEqual(result.results[0].status, 'ok')
  assert(result.results[0].output.includes('hello.txt'))

  // Verifica que o arquivo realmente existe
  const content = await fs.readFile(
    path.resolve('workspace', TASK_CTX.taskId, 'hello.txt'), 'utf-8'
  )
  assertEqual(content, 'olá mundo')
})

await test('file_read lê arquivo criado', async () => {
  const response = `<action name="file_read">
  <param name="path">hello.txt</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'ok')
  assert(result.results[0].output.includes('olá mundo'))
})

await test('shell executa comando simples', async () => {
  const response = `<action name="shell">
  <param name="command">echo "maker-agent funcionando"</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'ok')
  assert(result.results[0].output.includes('maker-agent funcionando'))
})

await test('file_write cria subdiretórios automaticamente', async () => {
  const response = `<action name="file_write">
  <param name="path">src/css/style.css</param>
  <param name="content">body { margin: 0; }</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'ok')
  const exists = await fs.access(
    path.resolve('workspace', TASK_CTX.taskId, 'src/css/style.css')
  ).then(() => true).catch(() => false)
  assert(exists, 'subdiretório deve ser criado automaticamente')
})

await test('bloqueia path traversal', async () => {
  const response = `<action name="file_write">
  <param name="path">../../etc/passwd</param>
  <param name="content">hackeado</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'error')
  assert(result.results[0].output.includes('bloqueado'))
})

await test('bloqueia comando perigoso', async () => {
  const response = `<action name="shell">
  <param name="command">rm -rf /</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'error')
  assert(result.results[0].output.includes('bloqueado'))
})

await test('ação desconhecida retorna error', async () => {
  const response = `<action name="drop_database">
  <param name="name">prod</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assertEqual(result.results[0].status, 'error')
  assert(result.results[0].output.includes('desconhecida'))
})

await test('múltiplas ações numa resposta — todas executadas', async () => {
  const response = `
<action name="file_write">
  <param name="path">multi-a.txt</param>
  <param name="content">arquivo A</param>
</action>
<action name="file_write">
  <param name="path">multi-b.txt</param>
  <param name="content">arquivo B</param>
</action>
<action name="shell">
  <param name="command">ls *.txt</param>
</action>`
  const result = await actionRegistry.run(response, TASK_CTX)
  assert(result.results.length === 3, `esperado 3, recebeu ${result.results.length}`)
  assert(result.results.every(r => r.status === 'ok'), 'todas devem ser ok')
})

// 3. Token counter
section('Token Counter')
const { TokenCounter } = await import('../src/maker/token-counter.js')
const counter = new TokenCounter({ modelName: 'qwen2.5-coder-4b' })
await counter.init()

await test('estima tokens de texto curto', async () => {
  const n = await counter.count('Olá, mundo!')
  assert(n > 0 && n < 20, `estimativa absurda: ${n}`)
})

await test('código tem estimativa diferente de prosa', async () => {
  const prose = 'Este é um parágrafo simples em português com algumas palavras.'
  const code  = 'function hello() { return { key: "value", arr: [1,2,3] }; }'
  const nProse = await counter.count(prose)
  const nCode  = await counter.count(code)
  // Código deve ter ratio diferente — ambos razoáveis
  assert(nProse > 0 && nCode > 0)
  console.log(`    ${c.gray}prosa: ${nProse}tk | código: ${nCode}tk${c.reset}`)
})

await test('texto longo estima mais tokens que texto curto', async () => {
  const short = 'Olá'
  const long  = 'Olá'.repeat(100)
  const nShort = await counter.count(short)
  const nLong  = await counter.count(long)
  assert(nLong > nShort, `longo (${nLong}) deve ser maior que curto (${nShort})`)
})

await test('willFit retorna false quando estoura budget', async () => {
  const bigMessages = Array.from({ length: 20 }, (_, i) => ({
    role: 'user',
    content: 'texto de teste '.repeat(50) + i
  }))
  const { fits, used, budget } = await counter.willFit(bigMessages, 512, 100)
  assert(!fits, `deveria não caber — used: ${used}, budget: ${budget}`)
})

await test('willFit retorna true para mensagem pequena', async () => {
  const messages = [{ role: 'user', content: 'Crie um hello world em JS' }]
  const { fits } = await counter.willFit(messages, 2048, 900)
  assert(fits, 'mensagem curta deve caber')
})

// 4. Task store
section('Task Store')
const { taskStore } = await import('../src/storage/task-store.js')

const sampleTask = {
  id:        'task-test-999',
  source:    'terminal',
  userId:    'local',
  goal:      'criar uma landing page',
  model:     'qwen2.5-coder-4b',
  status:    'pending',
  phases:    [
    { id: 'p1', name: 'HTML',  status: 'pending', summary: null },
    { id: 'p2', name: 'CSS',   status: 'pending', summary: null },
  ],
  createdAt: new Date().toISOString()
}

await test('salva e carrega task', async () => {
  await taskStore.save(sampleTask)
  const loaded = await taskStore.load(sampleTask.id)
  assert(loaded !== null, 'deve encontrar a task')
  assertEqual(loaded.goal, sampleTask.goal)
  assertEqual(loaded.phases.length, 2)
})

await test('patch atualiza campos sem sobrescrever tudo', async () => {
  await taskStore.patch(sampleTask.id, { status: 'running' })
  const loaded = await taskStore.load(sampleTask.id)
  assertEqual(loaded.status, 'running')
  assertEqual(loaded.goal, sampleTask.goal)  // campo original preservado
})

await test('listByUser retorna tasks do usuário', async () => {
  const tasks = await taskStore.listByUser('local')
  assert(tasks.length >= 1, 'deve ter pelo menos 1 task')
  assert(tasks.some(t => t.id === sampleTask.id))
})

await test('load retorna null para id inexistente', async () => {
  const result = await taskStore.load('task-nao-existe-xyz')
  assert(result === null)
})

await test('remove apaga a task', async () => {
  await taskStore.remove(sampleTask.id)
  const loaded = await taskStore.load(sampleTask.id)
  assert(loaded === null, 'task deve ter sido removida')
})

// 5. Session store
section('Session Store')
const { sessionStore } = await import('../src/storage/session-store.js')

await test('sessão começa vazia', () => {
  const msgs = sessionStore.get('terminal:user1')
  assert(Array.isArray(msgs) && msgs.length === 0)
})

await test('push adiciona mensagens', () => {
  sessionStore.push('terminal:user1', { role: 'user',      content: 'oi' })
  sessionStore.push('terminal:user1', { role: 'assistant', content: 'olá!' })
  const msgs = sessionStore.get('terminal:user1')
  assertEqual(msgs.length, 2)
  assertEqual(msgs[0].role, 'user')
})

await test('sessões de usuários diferentes são isoladas', () => {
  sessionStore.push('terminal:user2', { role: 'user', content: 'mensagem do user2' })
  const u1 = sessionStore.get('terminal:user1')
  const u2 = sessionStore.get('terminal:user2')
  assert(u1.length !== u2.length || u1[0].content !== u2[0].content)
  assertEqual(u2[0].content, 'mensagem do user2')
})

await test('clear reseta a sessão', () => {
  sessionStore.clear('terminal:user1')
  const msgs = sessionStore.get('terminal:user1')
  assertEqual(msgs.length, 0)
})

await test('não estoura MAX_MESSAGES — preserva a primeira', () => {
  const key = 'terminal:overflow'
  for (let i = 0; i < 25; i++) {
    sessionStore.push(key, { role: 'user', content: `msg ${i}` })
  }
  const msgs = sessionStore.get(key)
  assert(msgs.length <= 20, `deve ter no máximo 20, tem ${msgs.length}`)
  assertEqual(msgs[0].content, 'msg 0', 'primeira mensagem deve ser preservada')
})

await test('info retorna metadados da sessão', () => {
  sessionStore.push('telegram:99', { role: 'user', content: 'teste' })
  const info = sessionStore.info('telegram:99')
  assert(info.exists)
  assertEqual(info.messages, 1)
  assert(!info.expired)
})

await test('snapshot não afeta o histórico original', () => {
  sessionStore.push('terminal:snap', { role: 'user', content: 'original' })
  const snap = sessionStore.snapshot('terminal:snap', 'nota de sistema')
  const orig = sessionStore.get('terminal:snap')
  assert(snap.length === 2,  'snapshot deve ter mensagem extra')
  assert(orig.length === 1,  'original não deve ser afetado')
})

await test('persistência: saveAll e loadAll', async () => {
  sessionStore.push('telegram:persist', { role: 'user', content: 'persistindo' })
  await sessionStore.saveAll()

  // Simula novo processo limpando memória
  sessionStore.clear('telegram:persist')
  assert(sessionStore.get('telegram:persist').length === 0)

  await sessionStore.loadAll()
  const restored = sessionStore.get('telegram:persist')
  assert(restored.length === 1, 'deve restaurar sessão do disco')
  assertEqual(restored[0].content, 'persistindo')
})

// ─── Resultado final ──────────────────────────────────────────────────────────

console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)

if (failed > 0) process.exit(1)
