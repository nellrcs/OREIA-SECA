// src/maker/executor.js
import { prepareContext } from './context-builder.js'
import { actionRegistry } from '../actions/registry.js'
import { taskStore }      from '../storage/task-store.js'
import { config }         from '../config.js'

const sleep = ms => new Promise(r => setTimeout(r, ms))

const MAX_RETRIES  = 2
const RELOAD_DELAY = 2000

function extractSummary({ narrative, results }) {
  const files = results
    .filter(r => r.action === 'file_write' && r.status === 'ok')
    .map(r => r.output.match(/arquivo criado: ([\w./+-]+)/)?.[1])
    .filter(Boolean)

  const cmds = results
    .filter(r => r.action === 'shell' && r.status === 'ok')
    .map(r => r.output.split('\n')[0].replace('$ ', ''))

  const parts = []
  if (files.length) parts.push(`arquivos: ${files.join(', ')}`)
  if (cmds.length)  parts.push(`cmds: ${cmds.slice(0, 2).join(', ')}`)
  if (!parts.length) parts.push(narrative.split('\n')[0].slice(0, 120))

  return parts.join(' | ').slice(0, 200)
}

async function executePhase(phase, task, model, counter) {
  let lastError

  const modelContext = model.context || {}
  const globalContext = config.context || {}
  const maxTokens = modelContext.maxTokens ?? globalContext.maxTokens ?? 8192
  const reserveOutput = modelContext.reserveOutput ?? globalContext.reserveOutput ?? 2048

  const modelName = model.modelName ?? model.model ?? model.constructor.name ?? 'desconhecido'
  console.log(`[executor] Executando "${phase.name}" no modelo "${modelName}" com limite de ${maxTokens} tokens (reserva de ${reserveOutput})`)

  for (let attempt = 1; attempt <= MAX_RETRIES + 1; attempt++) {
    try {
      const messages = await prepareContext({
        goal:       task.goal,
        phase,
        donePhases: task.phases.filter(p => p.status === 'done'),
        maxTokens,
        reserveOutput,
      }, counter)

      const { text } = await model.generate(messages, { maxTokens: reserveOutput })
      const runResult = await actionRegistry.run(text, { taskId: task.id, userId: task.userId })
      return runResult

    } catch (err) {
      lastError = err
      if (attempt <= MAX_RETRIES) {
        console.warn(`[executor] "${phase.name}" tentativa ${attempt} falhou — retry`)
        await sleep(500 * attempt)
      }
    }
  }

  throw lastError
}

export async function executeTask(task, modelOrRouter, counter, input) {
  const isRouter = modelOrRouter && typeof modelOrRouter.forExecution === 'function'
  const executorModel = isRouter ? modelOrRouter.forExecution() : modelOrRouter

  // Garantir a criação da pasta do workspace e salvar o arquivo informativo sobre_a_tarefa.md
  const fs = await import('fs/promises')
  const path = await import('path')
  const workdir = path.resolve('workspace', task.id)
  await fs.mkdir(workdir, { recursive: true })

  const infoPath = path.join(workdir, 'sobre_a_tarefa.md')
  const phasesList = task.phases.map((p, i) => `${i + 1}. **${p.name}**\n   *Instrução:* ${p.instruction}`).join('\n\n')
  
  const infoContent = `# O.R.E.I.A.S.E.C.A — Detalhes da Tarefa

Este diretório contém o espaço de trabalho isolado para a execução da tarefa **${task.id}**.

## Objetivo Principal
> ${task.goal}

---

## Metadados da Execução
- **ID da Tarefa:** \`${task.id}\`
- **Criado em:** ${new Date(task.createdAt).toLocaleString('pt-BR')}
- **Modelo de IA:** \`${task.model}\`
- **Status Inicial:** \`${task.status}\`

---

## Fases Planejadas
Abaixo está o plano de etapas sequenciais gerado para cumprir o objetivo:

${phasesList}
`
  await fs.writeFile(infoPath, infoContent, 'utf-8')

  const pending = task.phases.filter(p => p.status === 'pending')

  for (const phase of pending) {
    await input.send(task.userId, `▸ ${phase.name}...`)

    try {
      const runResult = await executePhase(phase, task, executorModel, counter)

      phase.summary = extractSummary(runResult)
      phase.status  = 'done'
      await taskStore.save(task)

      const errs = runResult.results.filter(r => r.status === 'error')
      await input.send(task.userId, `${errs.length ? '⚠' : '✓'} ${phase.name}`)

      if (executorModel.reload) {
        await executorModel.reload()
        await sleep(RELOAD_DELAY)
      }

    } catch (err) {
      console.error(`[executor] "${phase.name}" falhou: ${err.message}`)
      phase.status  = 'failed'
      phase.summary = `ERRO: ${err.message.slice(0, 100)}`
      await taskStore.save(task)
      await input.send(task.userId, `✗ ${phase.name}: ${err.message}`)
    }
  }

  const failed = task.phases.filter(p => p.status === 'failed').length
  task.status = failed ? 'done_with_errors' : 'done'
  await taskStore.save(task)

  await input.send(task.userId, failed
    ? `✅ Concluído com ${failed} erro(s).`
    : `✅ Tarefa concluída!`
  )
}
