// src/actions/parser.js

export function parseActions(text) {
  const actions = []
  
  // Regex robusta para capturar <action name="nome"> ou <action name = 'nome' > etc.
  const actionRegex = /<action\s+name\s*=\s*["']?([\w_]+)["']?\s*>([\s\S]*?)<\/action>/gi
  let match

  while ((match = actionRegex.exec(text)) !== null) {
    const name   = match[1]
    const body   = match[2]
    const params = {}

    // Regex robusta para capturar <param name="nome"> ou <param name = 'nome' > etc.
    const paramRegex = /<param\s+name\s*=\s*["']?([\w_]+)["']?\s*>([\s\S]*?)<\/param>/gi
    let paramMatch
    while ((paramMatch = paramRegex.exec(body)) !== null) {
      params[paramMatch[1]] = paramMatch[2].trim()
    }

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
