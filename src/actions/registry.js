// src/actions/registry.js
import fs from 'fs/promises'
import path from 'path'
import { pathToFileURL } from 'url'
import { parseActions, parseNarrative } from './parser.js'
import { ShellAction }                  from './shell.js'
import { FileWriteAction, FileReadAction } from './filesystem.js'

const STATIC_ACTIONS = [ShellAction, FileWriteAction, FileReadAction]

class ActionRegistry {
  constructor() {
    this.handlers = new Map()
    this.registerStatic()
  }

  registerStatic() {
    this.handlers.clear()
    for (const ActionClass of STATIC_ACTIONS) {
      this.register(ActionClass)
    }
  }

  register(ActionClass) {
    this.handlers.set(ActionClass.actionName, new ActionClass())
    return this
  }

  async loadSkills(skillsDir = path.resolve('src/skills')) {
    this.registerStatic()

    try {
      await fs.mkdir(skillsDir, { recursive: true })
      const files = await fs.readdir(skillsDir)
      for (const file of files) {
        if (file.endsWith('.js')) {
          const filePath = path.join(skillsDir, file)
          const fileUrl = pathToFileURL(filePath).href + `?t=${Date.now()}`
          const { default: SkillClass } = await import(fileUrl)
          if (SkillClass && SkillClass.actionName) {
            this.register(SkillClass)
            console.log(`  [skills] carregada: ${SkillClass.actionName}`)
          }
        }
      }
    } catch (err) {
      console.error(`  [skills] erro ao carregar da pasta ${skillsDir}:`, err.message)
    }
  }

  getActionsSchema() {
    const schema = []
    for (const [name, handler] of this.handlers.entries()) {
      const Class = handler.constructor
      schema.push({
        name,
        description: Class.description || `Executes the ${name} action.`,
        params: Class.params || {}
      })
    }
    return schema
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
