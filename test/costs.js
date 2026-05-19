// test/costs.js
const c = {
  reset: '\x1b[0m', green: '\x1b[32m',
  red: '\x1b[31m',  cyan: '\x1b[36m', bold: '\x1b[1m', gray: '\x1b[90m'
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

function assert(cond, msg) { if (!cond) throw new Error(msg ?? 'assertion falhou') }
function assertClose(a, b, tol = 0.000001, msg) {
  if (Math.abs(a - b) > tol) throw new Error(msg ?? `esperado ~${b}, recebeu ${a}`)
}

// ─── Testes ───────────────────────────────────────────────────────────────────

section('Pricing — getPrice e calcCost')
const { getPrice, calcCost, formatUSD } = await import('../src/costs/pricing.js')

await test('getPrice — match exato', () => {
  const p = getPrice('gemini-2.5-flash')
  assert(p.input === 0.075 && p.output === 0.30)
})

await test('getPrice — match parcial (sufixo de versão)', () => {
  const p = getPrice('claude-sonnet-4-5-20250514')
  assert(p.input === 3.00 && p.output === 15.00)
})

await test('getPrice — modelo local retorna custo zero', () => {
  const p = getPrice('lmstudio')
  assert(p.input === 0 && p.output === 0)
})

await test('getPrice — modelo desconhecido lança erro', () => {
  let threw = false
  try { getPrice('modelo-fantasma-xyz') } catch { threw = true }
  assert(threw)
})

await test('calcCost — gemini-2.5-flash 1000 tokens entrada + 500 saída', () => {
  const cost = calcCost(
    { prompt_tokens: 1000, completion_tokens: 500 },
    'gemini-2.5-flash'
  )
  // (1000/1M * 0.075) + (500/1M * 0.30)
  // = 0.000075 + 0.00015 = 0.000225
  assertClose(cost, 0.000225)
})

await test('calcCost — claude-sonnet-4-5 é mais caro que gemini-flash', () => {
  const usage = { prompt_tokens: 1000, completion_tokens: 500 }
  const gemini  = calcCost(usage, 'gemini-2.5-flash')
  const claude  = calcCost(usage, 'claude-sonnet-4-5')
  assert(claude > gemini, `claude ${claude} deve ser > gemini ${gemini}`)
})

await test('calcCost — modelo local sempre zero', () => {
  const cost = calcCost(
    { prompt_tokens: 100000, completion_tokens: 50000 },
    'lmstudio'
  )
  assert(cost === 0)
})

await test('formatUSD — valores pequenos usam milicents', () => {
  const s = formatUSD(0.0000001)
  assert(s.includes('m'), `esperado sufixo 'm', recebeu: ${s}`)
})

await test('formatUSD — valores maiores usam centavos', () => {
  const s = formatUSD(0.05)
  assert(s.startsWith('$0.05'))
})

section('CostTracker — acumulação e budget')
const { CostTracker, BudgetExceededError } = await import('../src/costs/cost-tracker.js')

await test('track — acumula custo corretamente', () => {
  const t = new CostTracker({ modelName: 'gemini-2.5-flash', budget: null })
  t.track({ prompt_tokens: 1000, completion_tokens: 500 }, 'fase 1')
  t.track({ prompt_tokens: 800,  completion_tokens: 400 }, 'fase 2')
  assert(t.calls.length === 2)
  assert(t.spent > 0)
  // Custo deve ser maior que zero e razoável
  assertClose(t.spent, 0.000225 + 0.000180, 0.0001)
})

await test('track — modelo local não acumula custo', () => {
  const t = new CostTracker({ modelName: 'lmstudio', budget: null })
  t.track({ prompt_tokens: 50000, completion_tokens: 20000 }, 'fase')
  assert(t.spent === 0)
  assert(t.isFree)
})

await test('assertBudget — passa quando dentro do limite', () => {
  const t = new CostTracker({ modelName: 'gemini-2.5-flash', budget: 0.10 })
  t.track({ prompt_tokens: 100, completion_tokens: 50 }, 'fase 1')
  // Não deve lançar
  t.assertBudget(0.001)
  assert(true)
})

await test('assertBudget — lança BudgetExceededError ao estourar', () => {
  const t = new CostTracker({ modelName: 'claude-sonnet-4-5', budget: 0.001 })
  // Gasta quase tudo
  t.track({ prompt_tokens: 60, completion_tokens: 60 }, 'fase cara')

  let threw = false
  try {
    t.assertBudget(0.05)  // próxima fase projetaria acima do budget
  } catch (err) {
    threw = true
    assert(err instanceof BudgetExceededError)
    assert(err.message.includes('Budget excedido'))
    assert(err.budget === 0.001)
  }
  assert(threw, 'deve ter lançado BudgetExceededError')
})

await test('assertBudget — sem budget configurado nunca bloqueia', () => {
  const t = new CostTracker({ modelName: 'claude-opus-4-5', budget: null })
  // Simula milhares de tokens
  for (let i = 0; i < 10; i++) {
    t.track({ prompt_tokens: 100000, completion_tokens: 80000 }, `fase ${i}`)
  }
  t.assertBudget(999)  // não deve lançar
  assert(true)
})

await test('callback onWarn dispara ao atingir 80%', async () => {
  let warnFired = false
  const t = new CostTracker({
    modelName: 'claude-sonnet-4-5',
    budget:    0.0001,
    onWarn:    ({ percent }) => { warnFired = true; assert(percent >= 80) }
  })

  // Uma chamada grande que vai além dos 80%
  t.track({ prompt_tokens: 5, completion_tokens: 5 }, 'fase pesada')

  assert(warnFired, 'onWarn deve ter sido chamado')
})

await test('onWarn dispara apenas uma vez mesmo com múltiplas chamadas', () => {
  let warnCount = 0
  const t = new CostTracker({
    modelName: 'claude-sonnet-4-5',
    budget:    0.00001,
    onWarn:    () => { warnCount++ }
  })
  for (let i = 0; i < 5; i++) {
    t.track({ prompt_tokens: 10, completion_tokens: 10 }, `fase ${i}`)
  }
  assert(warnCount === 1, `onWarn deve disparar 1x, disparou ${warnCount}x`)
})

await test('summary — campos corretos', () => {
  const t = new CostTracker({ modelName: 'gemini-2.5-flash', budget: 0.05 })
  t.track({ prompt_tokens: 1000, completion_tokens: 500 }, 'fase 1')
  t.track({ prompt_tokens:  800, completion_tokens: 400 }, 'fase 2')

  const s = t.summary()
  assert(s.modelName === 'gemini-2.5-flash')
  assert(s.budget    === 0.05)
  assert(s.calls     === 2)
  assert(s.totalTokens.input  === 1800)
  assert(s.totalTokens.output ===  900)
  assert(s.percentUsed > 0 && s.percentUsed <= 100)
  assert(s.remaining >= 0)
})

await test('formatSummary — retorna string legível', () => {
  const t = new CostTracker({ modelName: 'gemini-2.5-flash', budget: 0.05 })
  t.track({ prompt_tokens: 1000, completion_tokens: 500 }, 'fase 1')
  const msg = t.formatSummary()
  assert(typeof msg === 'string')
  assert(msg.includes('Custo da tarefa'))
  assert(msg.includes('Tokens'))
  assert(msg.includes('Budget'))
})

// ─── Resultado ────────────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)
if (failed > 0) process.exit(1)
