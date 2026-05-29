// src/maker/planner.js
import { getPlannerSystemPrompt } from './prompts.js'
import { actionRegistry }        from '../actions/registry.js'

// Tenta extrair JSON válido de uma resposta que pode ter texto ao redor
function extractJSON(text) {

  // Tenta direto
  try { return JSON.parse(text.trim()) } catch { }

  // Tenta remover markdown code fences
  const stripped = text.replace(/```json|```/g, '').trim()
  try { return JSON.parse(stripped) } catch { }

  // Tenta extrair o primeiro bloco { ... } da resposta
  const match = text.match(/\{[\s\S]*\}/)
  if (match) {
    try { return JSON.parse(match[0]) } catch { }
  }

  return null
}

function validatePlan(plan) {



  if (!plan || !Array.isArray(plan.phases)) {
    throw new Error('Resposta não contém array "phases"')
  }
  if (plan.phases.length === 0) {
    throw new Error('Plano sem fases')
  }
  for (const p of plan.phases) {
    if (!p.id || !p.name || !p.instruction) {
      throw new Error(`Fase inválida: ${JSON.stringify(p)}`)
    }
  }
}

export async function planTask(goal, model) {
  const content = `Tarefa: ${goal}`
  const messages = [{ role: 'user', content }]

  const schemas = actionRegistry.getActionsSchema()
  const plannerSystemPrompt = getPlannerSystemPrompt(schemas)

  let lastError
  const MAX_RETRIES = 2

  for (let attempt = 1; attempt <= MAX_RETRIES; attempt++) {
    // Na segunda tentativa, reforça o formato no prompt
    const retryHint = attempt > 1
      ? [{ role: 'assistant', content: 'Vou retornar apenas o JSON solicitado.' }]
      : []

    const { text } = await model.generate(
      [...messages, ...retryHint],
      { system: plannerSystemPrompt, maxTokens: 2048 }
    )

    const plan = extractJSON(text)

    try {
      validatePlan(plan)
      // Adiciona status inicial a cada fase
      return plan.phases.slice(0, 8).map(p => ({
        ...p,
        status: 'pending',
        summary: null,
      }))
    } catch (err) {
      lastError = err
      console.warn(`[planner] tentativa ${attempt}/${MAX_RETRIES} — JSON inválido: ${err.message}`)
    }
  }

  throw new Error(`Planner falhou após ${MAX_RETRIES} tentativas: ${lastError.message}`)
}
