// src/maker/researcher.js
import { getResearcherSystemPrompt } from './prompts.js'
import { actionRegistry }           from '../actions/registry.js'

const MAX_RESEARCH_ITERATIONS = 2

/**
 * Executa uma análise automatizada do codebase para a tarefa desejada,
 * retornando um relatório técnico estruturado.
 * 
 * @param {string} goal          - O objetivo geral da tarefa
 * @param {ResilientModel} model - O modelo de IA resolvido para o papel 'researcher'
 * @returns {Promise<string>}    - O relatório técnico gerado
 */
export async function researchTask(goal, model) {
  const schemas = actionRegistry.getActionsSchema()
  const systemPrompt = getResearcherSystemPrompt(schemas)

  const messages = [
    {
      role: 'user',
      content: `Tarefa: "${goal}"\n\nPor favor, faça uma pesquisa técnica no codebase para planejar esta tarefa de forma ideal. Se necessário, execute ações de leitura e inspeção.`
    }
  ]

  console.log(`[researcher] 🔍 Iniciando investigação técnica do codebase para a tarefa: "${goal.slice(0, 60)}"`)

  for (let iter = 1; iter <= MAX_RESEARCH_ITERATIONS; iter++) {
    try {
      const response = await model.generate(messages, { system: systemPrompt })
      const text = response.text

      // Salva o pensamento na história da conversa
      messages.push({ role: 'assistant', content: text })

      // Verifica se o modelo solicitou ações
      const { results } = await actionRegistry.run(text, { taskId: 'research' })

      if (results && results.length > 0) {
        console.log(`[researcher] ⚙️ Iteração ${iter}: processando ${results.length} ações de pesquisa...`)
        
        // Formata os resultados e adiciona na história para a próxima iteração
        const formattedResults = actionRegistry.formatForContext({ results })
        messages.push({
          role: 'user',
          content: `Aqui estão os resultados das ações de pesquisa executadas:\n\n${formattedResults}`
        })
      } else {
        // Se não houver ações, a pesquisa está concluída
        console.log(`[researcher] ✓ Pesquisa concluída na iteração ${iter}.`)
        return extractResearchReport(text)
      }
    } catch (err) {
      console.error(`[researcher] ⚠️ Erro na iteração ${iter}:`, err.message)
      break
    }
  }

  // Se esgotar as iterações, extrai o relatório da última resposta gerada
  const lastMsg = messages[messages.length - 1]
  const lastText = lastMsg && lastMsg.role === 'assistant' ? lastMsg.content : 'Nenhum contexto adicional obtido.'
  return extractResearchReport(lastText)
}

/**
 * Extrai o bloco de relatório técnico delimitado da resposta do modelo.
 * Se os delimitadores não existirem, retorna o texto completo.
 */
function extractResearchReport(text) {
  const match = text.match(/=== TECHNICAL RESEARCH REPORT ===([\s\S]*?)=== END TECHNICAL RESEARCH REPORT ===/i)
  if (match) {
    return match[1].trim()
  }
  return text.trim()
}
