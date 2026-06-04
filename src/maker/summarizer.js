// src/maker/summarizer.js
import fs   from 'fs/promises'
import path from 'path'

const SUMMARIES_DIR = path.resolve('resumos-contextos')

// ─── Garante que o diretório existe ──────────────────────────────────────────

async function ensureDir() {
  await fs.mkdir(SUMMARIES_DIR, { recursive: true })
}

// ─── Gera o resumo via LLM ────────────────────────────────────────────────────

/**
 * Pede ao modelo um resumo compacto do histórico de mensagens.
 * @param {Array<{role:string, content:string}>} messages  — histórico da sessão
 * @param {object} model  — instância de modelo (ResilientModel ou BaseModel)
 * @returns {Promise<string>}  — texto do resumo em pt-BR
 */
export async function summarizeSession(messages, model) {
  if (!messages.length) return ''

  const conversation = messages
    .map(m => `${m.role === 'assistant' ? 'Assistente' : 'Usuário'}: ${m.content}`)
    .join('\n\n')

  const prompt = [
    { role: 'user', content:
      `A seguir está uma conversa que precisa ser comprimida em um resumo compacto.\n\n` +
      `CONVERSA:\n${conversation}\n\n` +
      `Escreva um resumo em 2-4 parágrafos em português (pt-BR) que capture:\n` +
      `- O contexto principal e o tema da conversa\n` +
      `- Decisões, fatos ou resultados importantes mencionados\n` +
      `- O estado atual (o que foi concluído, o que está pendente)\n\n` +
      `Seja objetivo. Não use títulos ou marcadores — escreva em prosa corrida.`
    }
  ]

  const { text } = await model.generate(prompt, { maxTokens: 512, temperature: 0.2 })
  return text.trim()
}

// ─── Salva o resumo em disco como .md ────────────────────────────────────────

/**
 * Persiste o resumo em `resumos-contextos/<sessionKey>_<timestamp>.md`
 * e retorna o caminho do arquivo criado.
 * @param {string} sessionKey  — ex: "terminal:local" ou "telegram:12345"
 * @param {string} summary     — texto do resumo gerado
 * @returns {Promise<string>}  — caminho absoluto do arquivo salvo
 */
export async function saveSummaryFile(sessionKey, summary) {
  await ensureDir()

  const ts        = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19)
  const safeName  = sessionKey.replace(/[^a-zA-Z0-9_-]/g, '_')
  const filename  = `${safeName}_${ts}.md`
  const filepath  = path.join(SUMMARIES_DIR, filename)

  const content = [
    `# Resumo de Contexto`,
    ``,
    `**Sessão:** \`${sessionKey}\`  `,
    `**Gerado em:** ${new Date().toLocaleString('pt-BR')}`,
    ``,
    `---`,
    ``,
    summary,
    ``,
    `---`,
    ``,
    `> Para retomar esta conversa use: \`/carregar ${filename}\``,
  ].join('\n')

  await fs.writeFile(filepath, content, 'utf-8')
  return filepath
}

// ─── Carrega um resumo de um arquivo .md ─────────────────────────────────────

/**
 * Lê um arquivo de resumo e retorna o texto de contexto pronto para ser
 * injetado na sessão como mensagem inicial.
 * @param {string} filename  — nome do arquivo (com ou sem caminho)
 * @returns {Promise<string|null>}
 */
export async function loadSummaryFile(filename) {
  await ensureDir()

  // Aceita apenas o nome do arquivo (sem path) por segurança
  const basename = path.basename(filename)
  const filepath = path.join(SUMMARIES_DIR, basename)

  try {
    const raw = await fs.readFile(filepath, 'utf-8')

    // Extrai apenas o corpo do resumo (entre os dois "---")
    const match = raw.match(/---\n\n([\s\S]+?)\n\n---/)
    return match ? match[1].trim() : raw.trim()
  } catch {
    return null
  }
}

// ─── Lista os resumos disponíveis ─────────────────────────────────────────────

/**
 * Retorna a lista de arquivos .md em resumos-contextos/, do mais recente ao mais antigo.
 * @returns {Promise<string[]>}  — lista de nomes de arquivo
 */
export async function listSummaryFiles() {
  await ensureDir()
  try {
    const files = await fs.readdir(SUMMARIES_DIR)
    return files
      .filter(f => f.endsWith('.md'))
      .sort()
      .reverse()  // mais recentes primeiro
  } catch {
    return []
  }
}
