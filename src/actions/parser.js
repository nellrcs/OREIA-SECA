// src/actions/parser.js

export function parseActions(text) {
  const actions = []
  const actionRegex = /<action\s+name="([\w_]+)">([\s\S]*?)<\/action>/g
  let match

  while ((match = actionRegex.exec(text)) !== null) {
    const name   = match[1]
    const body   = match[2]
    const params = {}

    const paramRegex = /<param\s+name="([\w_]+)">([\s\S]*?)<\/param>/g
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
    .replace(/<action[\s\S]*?<\/action>/g, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}
