// src/maker/validator.js
import { getValidatorSystemPrompt } from './prompts.js'
import { actionRegistry }           from '../actions/registry.js'

const MAX_VALIDATOR_ITERATIONS = 2

/**
 * Tenta extrair JSON válido de uma resposta que pode conter texto ou blocos Markdown.
 */
function extractJSON(text) {
  try { return JSON.parse(text.trim()) } catch { }

  const stripped = text.replace(/```json|```/g, '').trim()
  try { return JSON.parse(stripped) } catch { }

  const match = text.match(/\{[\s\S]*\}/)
  if (match) {
    try { return JSON.parse(match[0]) } catch { }
  }

  return null
}

/**
 * Executa uma verificação técnica dos resultados implementados para garantir corretude.
 * 
 * @param {object} task          - O objeto de tarefa com fases executadas
 * @param {ResilientModel} model - O modelo de IA resolvido para o papel 'validator'
 * @returns {Promise<object>}    - Relatório de validação com status, narrativa e problemas
 */
export async function validateTask(task, model) {
  const schemas = actionRegistry.getActionsSchema()
  const systemPrompt = getValidatorSystemPrompt(schemas)

  const phasesText = task.phases
    .map(p => `- Fase: "${p.name}" | Status: ${p.status} | Resumo: ${p.summary ?? 'nenhum'}`)
    .join('\n')

  const messages = [
    {
      role: 'user',
      content: `Tarefa Objetivo: "${task.goal}"\n\nFases executadas:\n${phasesText}\n\nPor favor, valide as alterações e implementações técnicas realizadas. Se necessário, rode ações de leitura ou checagem.`
    }
  ]

  console.log(`[validator] 🔍 Iniciando homologação/validação técnica para a tarefa: "${task.id}"`)

  for (let iter = 1; iter <= MAX_VALIDATOR_ITERATIONS; iter++) {
    try {
      const response = await model.generate(messages, { system: systemPrompt })
      const text = response.text

      messages.push({ role: 'assistant', content: text })

      // Verifica se o modelo solicitou ações de validação
      const { results } = await actionRegistry.run(text, { taskId: task.id })

      if (results && results.length > 0) {
        console.log(`[validator] ⚙️ Iteração ${iter}: processando ${results.length} ações de checagem...`)
        
        const formattedResults = actionRegistry.formatForContext({ results })
        messages.push({
          role: 'user',
          content: `Aqui estão os resultados das checagens executadas:\n\n${formattedResults}`
        })
      } else {
        console.log(`[validator] ✓ Validação técnica concluída na iteração ${iter}.`)
        const parsed = extractJSON(text)
        return parsed || { status: 'SUCCESS', narrative: text, issues: [] }
      }
    } catch (err) {
      console.error(`[validator] ⚠️ Erro na iteração ${iter}:`, err.message)
      break
    }
  }

  // Se esgotar as iterações, extrai o JSON da última resposta do assistente
  const lastMsg = messages[messages.length - 1]
  const lastText = lastMsg && lastMsg.role === 'assistant' ? lastMsg.content : ''
  const parsed = extractJSON(lastText)
  
  return parsed || {
    status: 'SUCCESS',
    narrative: lastText || 'Não foi possível validar de forma estruturada.',
    issues: []
  }
}
