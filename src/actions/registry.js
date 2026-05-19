// src/actions/registry.js
import { parseActions, parseNarrative } from './parser.js'
import { ShellAction }                  from './shell.js'
import { FileWriteAction, FileReadAction } from './filesystem.js'

class ActionRegistry {
  constructor() {
    this.handlers = new Map()
  }

  register(ActionClass) {
    this.handlers.set(ActionClass.actionName, new ActionClass())
    return this
  }

  async run(modelResponse, context) {
    const actions   = parseActions(modelResponse)
    const narrative = parseNarrative(modelResponse)
    const results   = []

    for (const action of actions) {
      const handler = this.handlers.get(action.name)

      if (!handler) {
        results.push({ action: action.name, status: 'error', output: `ação desconhecida: "${action.name}"` })
        continue
      }

      console.log(`  [action] ${action.name}`, JSON.stringify(action.params).slice(0, 80))

      try {
        const output = await handler.run(action.params, context)
        results.push({ action: action.name, status: 'ok', output })
      } catch (err) {
        results.push({ action: action.name, status: 'error', output: err.message })
        console.error(`  [action] erro em ${action.name}: ${err.message}`)
      }
    }

    return { narrative, results }
  }

  formatForContext({ narrative, results }) {
    if (!results.length) return narrative || '(fase concluída sem ações)'

    const blocks = results.map(r =>
      `<result action="${r.action}" status="${r.status}">\n${r.output}\n</result>`
    )
    return [narrative, ...blocks].filter(Boolean).join('\n\n')
  }
}

export const actionRegistry = new ActionRegistry()
  .register(ShellAction)
  .register(FileWriteAction)
  .register(FileReadAction)
