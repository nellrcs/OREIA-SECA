// test/run.js — MASTER TEST RUNNER
import { fork } from 'child_process'
import path from 'path'
import { fileURLToPath } from 'url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

const c = {
  reset:  '\x1b[0m',
  green:  '\x1b[32m',
  red:    '\x1b[31m',
  yellow: '\x1b[33m',
  cyan:   '\x1b[36m',
  gray:   '\x1b[90m',
  bold:   '\x1b[1m',
}

const testSuites = [
  { name: 'Unit Tests', file: 'unit.js' },
  { name: 'Core Tests', file: 'core.js' },
  { name: 'Boot Tests', file: 'boot.js' },
  { name: 'Integration Tests', file: 'integration.js' },
  { name: 'Approval & REST Tests', file: 'approval-rest.js' },
  { name: 'Skills Tests', file: 'skills.js' },
]

function runSuite(suite) {
  return new Promise((resolve) => {
    console.log(`\n${c.yellow}${c.bold}🚀 Executando suíte: ${suite.name} (${suite.file})${c.reset}`)
    console.log(`${c.gray}${'─'.repeat(50)}${c.reset}`)

    const child = fork(path.join(__dirname, suite.file), [], {
      stdio: 'inherit' // Permite herdar cores e logs em tempo real
    })

    child.on('close', (code) => {
      const passed = code === 0
      resolve({ ...suite, passed, code })
    })
  })
}

async function main() {
  console.log(`
${c.cyan}${c.bold}╔══════════════════════════════════════════════════════════╗
║                    MASTER TEST RUNNER                    ║
║                   Executando toda a suíte                ║
╚══════════════════════════════════════════════════════════╝${c.reset}
`)

  const results = []
  
  for (const suite of testSuites) {
    const res = await runSuite(suite)
    results.push(res)
  }

  console.log(`\n\n${c.bold}═══ RESUMO GERAL DAS SUÍTES ═══${c.reset}`)
  console.log(`${c.gray}${'═'.repeat(40)}${c.reset}`)

  let allPassed = true
  
  for (const res of results) {
    if (res.passed) {
      console.log(`  ${c.green}✓ ${c.bold}${res.name.padEnd(25)}${c.reset} ${c.green}PASSOU${c.reset}`)
    } else {
      console.log(`  ${c.red}✗ ${c.bold}${res.name.padEnd(25)}${c.reset} ${c.red}FALHOU (código ${res.code})${c.reset}`)
      allPassed = false
    }
  }

  console.log(`${c.gray}${'═'.repeat(40)}${c.reset}`)
  
  if (allPassed) {
    console.log(`\n  ${c.green}${c.bold}🎉 Sucesso! Todas as suítes de teste passaram!${c.reset}\n`)
    process.exit(0)
  } else {
    console.log(`\n  ${c.red}${c.bold}❌ Falha! Uma ou mais suítes de teste falharam.${c.reset}\n`)
    process.exit(1)
  }
}

main().catch(err => {
  console.error('Erro no test runner:', err)
  process.exit(1)
})
