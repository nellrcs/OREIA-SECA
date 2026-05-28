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

await test('carrega e executa skill Markdown (.md) com sucesso', async () => {
  const tempMdPath = path.resolve('src/skills/skill_temp_math.md')
  
  const mdContent = `# skill_temp_math

Executes a dynamic mathematical sum.

## Parameters
- \`a\` (number, required): The first number
- \`b\` (number, required): The second number

## Code
\`\`\`javascript
const result = Number(a) + Number(b);
return \`O resultado de \${a} + \${b} é \${result}\`;
\`\`\`
`

  await fs.writeFile(tempMdPath, mdContent, 'utf8')

  try {
    // Recarrega as skills
    await actionRegistry.loadSkills()

    // Verifica se registrou a nova skill
    const handler = actionRegistry.handlers.get('skill_temp_math')
    assert(!!handler, 'deve encontrar o handler de skill_temp_math')
    assertEqual(handler.constructor.description, 'Executes a dynamic mathematical sum.')
    assertEqual(handler.constructor.params.a.type, 'number')
    assertEqual(handler.constructor.params.a.required, true)
    
    // Executa a skill
    const res = await handler.run({ a: 10, b: 20 }, { taskId: 'test' })
    assertEqual(res, 'O resultado de 10 + 20 é 30')
  } finally {
    // Remove o arquivo temporário
    await fs.unlink(tempMdPath).catch(() => {})
  }
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

section('Agendador Cron Dinâmico (Docker/Local)')

await test('skill_cron_manager gerencia rotinas, faz parse textual de horários e gera arquivos crontab', async () => {
  const handler = actionRegistry.handlers.get('skill_cron_manager')
  assert(!!handler, 'handler do cron manager deve estar disponível')

  const testRoutineId = 'test_weather_daily'
  
  // 1. Cadastra uma nova rotina usando linguagem natural
  const addRes = await handler.run({
    action: 'add',
    routineId: testRoutineId,
    promptText: 'consulte o clima',
    textSchedule: 'todos os dias as 14 horas'
  }, { taskId: 'test' })

  assert(addRes.includes('sucesso') || addRes.includes('salva localmente'), 'deve retornar mensagem de sucesso ou salvamento local')

  // 2. Valida se salvou no JSON de rotinas
  const ROUTINES_FILE = path.resolve('tasks/cron_routines.json')
  const routinesContent = await fs.readFile(ROUTINES_FILE, 'utf8')
  const routines = JSON.parse(routinesContent)
  
  assert(!!routines[testRoutineId], 'a rotina de teste deve existir no JSON')
  assertEqual(routines[testRoutineId].cron, '0 14 * * *', 'a expressão cron para 14 horas deve ser parsed corretamente como 0 14 * * *')
  assertEqual(routines[testRoutineId].promptText, 'consulte o clima')

  // 3. Valida se gerou o arquivo de crontab correto
  const CRONTAB_FILE = path.resolve('tasks/crontab')
  const crontabContent = await fs.readFile(CRONTAB_FILE, 'utf8')
  assert(crontabContent.includes('0 14 * * *'), 'crontab deve conter o agendamento de 14 horas')
  assert(crontabContent.includes('http://host.docker.internal:3120/api/message'), 'crontab deve chamar a API REST')

  // 4. Testa a listagem das rotinas
  const listRes = await handler.run({ action: 'list' }, { taskId: 'test' })
  assert(listRes.includes(testRoutineId), 'a listagem deve conter a rotina de teste')

  // 5. Remove a rotina cadastrada para limpar o ambiente de testes
  const removeRes = await handler.run({
    action: 'remove',
    routineId: testRoutineId
  }, { taskId: 'test' })

  assert(removeRes.includes('sucesso') || removeRes.includes('localmente'), 'deve retornar mensagem de sucesso na remoção')

  // 6. Confere se foi removido do JSON
  const finalContent = await fs.readFile(ROUTINES_FILE, 'utf8')
  const finalRoutines = JSON.parse(finalContent)
  assert(!finalRoutines[testRoutineId], 'a rotina de teste deve ter sido removida do JSON')
})

await test('actionRegistry carrega e executa skill declarada em subpasta (via skill.md)', async () => {
  const tempSkillDirPath = path.resolve('src/skills/skill_temp_folder')
  const tempMdPath = path.join(tempSkillDirPath, 'skill.md')

  // 1. Cria a subpasta
  await fs.mkdir(tempSkillDirPath, { recursive: true })

  const mdContent = `# skill_temp_folder

Prints a dynamic welcome message for folder testing.

## Parameters
- \`name\` (string, required): The name of the user

## Code
\`\`\`javascript
return \`Bem-vindo à skill em pasta, \${name}!\`;
\`\`\`
`

  // 2. Escreve o arquivo skill.md na subpasta
  await fs.writeFile(tempMdPath, mdContent, 'utf8')

  try {
    // 3. Recarrega as skills
    await actionRegistry.loadSkills()

    // 4. Confere se carregou
    const handler = actionRegistry.handlers.get('skill_temp_folder')
    assert(!!handler, 'deve encontrar o handler de skill_temp_folder')
    assertEqual(handler.constructor.description, 'Prints a dynamic welcome message for folder testing.')
    assertEqual(handler.constructor.params.name.required, true)

    // 5. Executa
    const res = await handler.run({ name: 'Maria' }, { taskId: 'test-folder' })
    assertEqual(res, 'Bem-vindo à skill em pasta, Maria!')
  } finally {
    // 6. Limpeza completa dos arquivos temporários
    await fs.unlink(tempMdPath).catch(() => {})
    await fs.rmdir(tempSkillDirPath).catch(() => {})
  }
})

// ─── Resultado final ──────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)

if (failed > 0) process.exit(1)
process.exit(0)
