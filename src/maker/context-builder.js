// src/maker/context-builder.js
import { getExecutorSystemPrompt } from './prompts.js'
import { actionRegistry } from '../actions/registry.js'

const MAX_CONTEXT    = 8192   // Qwen3.5-9B suporta 128K — 8K é conservador e seguro
const RESERVE_OUTPUT = 2048  // espaço para o modelo gerar código completo
const INPUT_BUDGET   = MAX_CONTEXT - RESERVE_OUTPUT  // 6144 tokens disponíveis para input

// ─── Monta as mensagens para uma fase ────────────────────────────────────────

function buildMessages({ goal, phase, history }) {
  const historyBlock = history.length
    ? '\nJÁ CONCLUÍDO:\n' + history
        .filter(p => p.summary)
        .map(p => `- ${p.name}: ${p.summary}`)
        .join('\n')
    : ''

  const content = [
    `OBJETIVO: ${goal}`,
    historyBlock,
    `\nFASE ATUAL — ${phase.name}:`,
    phase.instruction,
    '\nExecute apenas esta fase.',
  ].filter(Boolean).join('\n')

  const systemPrompt = getExecutorSystemPrompt(actionRegistry.getActionsSchema())

  return [
    { role: 'user', content: systemPrompt + '\n\n' + content }
  ]
}

// ─── Comprime histórico numa linha (~20 tokens) ───────────────────────────────

function compressHistory(phases) {
  return phases
    .filter(p => p.summary)
    .map(p => {
      const files = (p.summary.match(/[\w-]+\.\w{1,5}/g) || []).join(', ')
      const short = files || p.summary.split(' ').slice(0, 6).join(' ')
      return `${p.name}: ${short}`
    })
    .join(' | ')
}

// ─── Trunca instrução preservando a primeira frase ───────────────────────────

function truncateInstruction(phase, targetChars) {
  const sentences = phase.instruction.split(/(?<=[.!?])\s+/).filter(Boolean)
  let result = ''
  for (const s of sentences) {
    if ((result + s).length > targetChars) break
    result = result ? `${result} ${s}` : s
  }
  return { ...phase, instruction: result || sentences[0] }
}

// ─── truncateHistory: 4 estratégias em cascata ───────────────────────────────

async function truncateHistory(goal, phase, donePhases, counter, maxTokens, reserveOutput) {
  let history = [...donePhases]

  // Estratégia 1: remove fases mais antigas uma a uma
  while (history.length > 0) {
    const msgs = buildMessages({ goal, phase, history })
    const { fits } = await counter.willFit(msgs, maxTokens, reserveOutput)
    if (fits) {
      console.log(`[context] s1 — histórico reduzido para ${history.length} fase(s)`)
      return msgs
    }
    history.shift()
  }

  // Estratégia 2: comprime todo o histórico numa linha
  const compressed = compressHistory(donePhases)
  if (compressed) {
    const msgs = buildMessages({ goal, phase, history: [{ name: 'resumo', summary: compressed }] })
    const { fits } = await counter.willFit(msgs, maxTokens, reserveOutput)
    if (fits) {
      console.log('[context] s2 — histórico comprimido numa linha')
      return msgs
    }
  }

  // Estratégia 3: sem histórico
  {
    const msgs = buildMessages({ goal, phase, history: [] })
    const { fits } = await counter.willFit(msgs, maxTokens, reserveOutput)
    if (fits) {
      console.log('[context] s3 — histórico removido')
      return msgs
    }
  }

  // Estratégia 4: trunca a instrução (último recurso)
  const truncated = truncateInstruction(phase, 300)
  console.warn('[context] s4 — instrução truncada (budget muito pequeno)')
  return buildMessages({ goal, phase: truncated, history: [] })
}

// ─── Entry point ─────────────────────────────────────────────────────────────

export async function prepareContext({ goal, phase, donePhases, maxTokens = 8192, reserveOutput = 2048 }, counter) {
  const full = buildMessages({ goal, phase, history: donePhases })
  const { fits, used, budget } = await counter.willFit(full, maxTokens, reserveOutput)

  if (fits) {
    console.log(`[context] ok — ${used}/${budget} tokens`)
    return full
  }

  console.warn(`[context] ${used} > ${budget} tokens — truncando`)
  return truncateHistory(goal, phase, donePhases, counter, maxTokens, reserveOutput)
}
