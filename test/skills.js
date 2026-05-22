// test/skills.js
import path from 'path'
import fs   from 'fs/promises'
import { actionRegistry } from '../src/actions/registry.js'
import { getExecutorSystemPrompt, getDirectSystemPrompt } from '../src/maker/prompts.js'

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

section('Carregamento de Habilidades Dinâmicas (Skills)')

await test('carrega skills padrão com sucesso', async () => {
  // Inicialmente tem as 3 estáticas
  assertEqual(actionRegistry.handlers.has('shell'), true, 'deve ter shell')
  assertEqual(actionRegistry.handlers.has('file_write'), true, 'deve ter file_write')
  assertEqual(actionRegistry.handlers.has('file_read'), true, 'deve ter file_read')

  // Carrega dinâmicas
  await actionRegistry.loadSkills()

  assertEqual(actionRegistry.handlers.has('skill_fetch'), true, 'deve ter carregado skill_fetch')
  assertEqual(actionRegistry.handlers.has('skill_weather'), true, 'deve ter carregado skill_weather')
})

await test('getActionsSchema retorna schemas válidos com descrições e parâmetros em inglês', () => {
  const schemas = actionRegistry.getActionsSchema()
  assert(schemas.length >= 5, `esperado pelo menos 5 ações registradas, recebeu ${schemas.length}`)

  const skillFetch = schemas.find(s => s.name === 'skill_fetch')
  assert(!!skillFetch, 'deve encontrar o schema de skill_fetch')
  assertEqual(skillFetch.description, 'Fetches raw text or HTML content from a public URL via HTTP request.')
  assertEqual(skillFetch.params.url.required, true)
  assertEqual(skillFetch.params.url.type, 'string')
})

await test('getExecutorSystemPrompt e getDirectSystemPrompt injetam metadados em inglês', () => {
  const schemas = actionRegistry.getActionsSchema()
  const execPrompt = getExecutorSystemPrompt(schemas)
  const directPrompt = getDirectSystemPrompt(schemas)

  // Verifica presença de termos em inglês do prompt do executor
  assert(execPrompt.includes('AVAILABLE ACTIONS:'), 'prompt do executor deve conter AVAILABLE ACTIONS')
  assert(execPrompt.includes('skill_fetch'), 'prompt deve conter a skill_fetch')
  assert(execPrompt.includes('skill_weather'), 'prompt deve conter a skill_weather')
  assert(execPrompt.includes('Brazilian Portuguese (pt-BR)'), 'deve instruir para responder em pt-BR')

  // Verifica prompt direto
  assert(directPrompt.includes('AVAILABLE ACTIONS:'), 'prompt direto deve conter AVAILABLE ACTIONS')
  assert(directPrompt.includes('skill_fetch'), 'prompt direto deve conter skill_fetch')
})

section('Segurança e Isolamento de Skills')

await test('skill_fetch bloqueia conexões locais por segurança', async () => {
  const handler = actionRegistry.handlers.get('skill_fetch')
  assert(!!handler, 'handler do fetch deve estar disponível')

  try {
    await handler.run({ url: 'http://localhost:3000' }, { taskId: 'test' })
    assert(false, 'deveria ter falhado para localhost')
  } catch (err) {
    assert(err.message.includes('blocked for security reasons'), 'deve indicar bloqueio de segurança')
  }

  try {
    await handler.run({ url: 'http://127.0.0.1/abc' }, { taskId: 'test' })
    assert(false, 'deveria ter falhado para 127.0.0.1')
  } catch (err) {
    assert(err.message.includes('blocked for security reasons'), 'deve indicar bloqueio de segurança')
  }
})

await test('skill_weather executa com sucesso e retorna condições meteorológicas', async () => {
  const handler = actionRegistry.handlers.get('skill_weather')
  assert(!!handler, 'handler do clima deve estar disponível')

  const res = await handler.run({ city: 'Sao Paulo' }, { taskId: 'test' })
  assert(res.includes('Weather for Sao Paulo'), 'deve retornar dados do clima')
})

// ─── Resultado final ──────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)

if (failed > 0) process.exit(1)
process.exit(0)
