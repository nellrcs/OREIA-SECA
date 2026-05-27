// src/actions/parser.js

export function parseActions(text) {
  const actions = []
  
  // Regex robusta para capturar <action name="nome"> ou <action name = 'nome' > etc.
  // Suporta outros atributos antes ou depois do name.
  const actionRegex = /<action\s+[^>]*name\s*=\s*["'“‘]?([\w_]+)["'”’]?[^>]*>([\s\S]*?)<\/action>/gi
  let match

  while ((match = actionRegex.exec(text)) !== null) {
    const name   = match[1]
    const body   = match[2]
    const params = {}

    // ─── Estratégia 1: Parsear atributos da tag de abertura <action> ───────────
    const openingTag = match[0].split('>')[0]
    // Captura pares key="value", key='value' ou key=value de forma flexível
    const attrRegex = /([\w_]+)\s*=\s*(?:["'“‘]([^"'”’>]+)["'”’]|([^\s>]+))/g
    let attrMatch
    while ((attrMatch = attrRegex.exec(openingTag)) !== null) {
      const key = attrMatch[1].toLowerCase()
      const val = attrMatch[2] || attrMatch[3]
      if (key !== 'name' && val) {
        params[key] = val.trim()
      }
    }

    // ─── Estratégia 2: Standard <param name="xxx">value</param> ─────────────────
    // Suporta aspas duplas, simples, curlas ou sem aspas
    const paramRegex = /<param\s+name\s*=\s*["'“‘]?([\w_]+)["'”’]?\s*>([\s\S]*?)<\/param>/gi
    let paramMatch
    while ((paramMatch = paramRegex.exec(body)) !== null) {
      params[paramMatch[1]] = paramMatch[2].trim()
    }

    // ─── Estratégia 3: Tags diretas de parâmetros (ex: <path>excluir.php</path>) ──
    // Captura <tag>valor</tag> exceto se for "param"
    const directTagRegex = /<([\w_]+)\s*>([\s\S]*?)<\/([\w_]+)>/gi
    let directMatch
    while ((directMatch = directTagRegex.exec(body)) !== null) {
      const tagName = directMatch[1].toLowerCase()
      const closingTagName = directMatch[3].toLowerCase()
      if (tagName === closingTagName && tagName !== 'param') {
        params[tagName] = directMatch[2].trim()
      }
    }

    // ─── Estratégia 4: Formato Key-Value linhas de texto (ex: path: excluir.php) ──
    const lines = body.split('\n')
    for (const line of lines) {
      const kvMatch = line.match(/^\s*([\w_]+)\s*[:=]\s*(.+)$/)
      if (kvMatch) {
        const key = kvMatch[1].trim().toLowerCase()
        const val = kvMatch[2].trim()
        // Remove aspas
        const cleanVal = val.replace(/^["'“‘]|["'”’]$/g, '')
        if (!params[key] && key !== 'action' && key !== 'param') {
          params[key] = cleanVal
        }
      }
    }

    // ─── Estratégia 5: JSON completo no corpo ─────────────────────────────────
    try {
      const json = JSON.parse(body.trim())
      if (json && typeof json === 'object') {
        Object.assign(params, json)
      }
    } catch (e) {}

    actions.push({ name, params })
  }

  return actions
}

export function parseNarrative(text) {
  return text
    .replace(/<action[\s\S]*?<\/action>/gi, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}
