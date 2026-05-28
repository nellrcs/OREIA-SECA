// src/actions/registry.js
import fs from 'fs/promises'
import path from 'path'
import { pathToFileURL } from 'url'
import { parseActions, parseNarrative } from './parser.js'
import { ShellAction }                  from './shell.js'
import { FileWriteAction, FileReadAction } from './filesystem.js'
import { BaseAction }                   from './base-action.js'

const STATIC_ACTIONS = [ShellAction, FileWriteAction, FileReadAction]

function parseMarkdownSkill(mdContent) {
  // 1. Nome da Skill (ex: # skill_weather)
  const nameMatch = mdContent.match(/^#\s+([\w_]+)/m)
  if (!nameMatch) return null
  const name = nameMatch[1].trim()

  // 2. Descrição (texto entre o título principal e a próxima seção ##)
  const descMatch = mdContent.match(/(?:^|\n)#\s+[\w_]+\s*\n+([\s\S]*?)(?=\n##|$)/)
  const description = descMatch ? descMatch[1].trim() : ''

  // 3. Parâmetros (extrai da seção ## Parameters ou ## Parâmetros)
  const params = {}
  const paramsMatch = mdContent.match(/(?:^|\n)##\s*(?:Parameters|Parâmetros)\s*\n+([\s\S]*?)(?=\n##|$)/i)
  const paramsText = paramsMatch ? paramsMatch[1] : ''
  
  if (paramsText) {
    const lines = paramsText.split('\n')
    for (const line of lines) {
      // Formato: - `nome` (tipo, obrigatório/opcional): descrição
      const match = line.match(/^-\s+`([\w_]+)`\s*\(([^)]+)\)\s*:\s*(.+)$/)
      if (match) {
        const pName = match[1]
        const pMetaRaw = match[2].toLowerCase()
        const pDesc = match[3].trim()
        
        const isRequired = pMetaRaw.includes('required') || pMetaRaw.includes('obrigatório')
        const type = pMetaRaw.includes('number') || pMetaRaw.includes('número') ? 'number'
                   : pMetaRaw.includes('boolean') || pMetaRaw.includes('booleano') ? 'boolean'
                   : 'string'
        
        params[pName] = {
          type,
          description: pDesc,
          required: isRequired
        }
      }
    }
  }

  // 4. Código JS (extrai o bloco ```javascript ou ```js)
  const codeMatch = mdContent.match(/```(?:javascript|js)\n([\s\S]*?)```/)
  if (!codeMatch) return null
  const codeString = codeMatch[1].trim()

  // 5. Criação dinâmica da Classe
  const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor
  const paramKeys = Object.keys(params)
  
  let runFn
  if (paramKeys.length > 0) {
    runFn = new AsyncFunction('params', 'context', `
      const { ${paramKeys.join(', ')} } = params;
      ${codeString}
    `)
  } else {
    runFn = new AsyncFunction('params', 'context', codeString)
  }

  return class extends BaseAction {
    static actionName = name
    static description = description
    static params = params

    async run(args, context) {
      // Validação rápida de parâmetros obrigatórios
      for (const [pName, pMeta] of Object.entries(params)) {
        if (pMeta.required && args[pName] === undefined) {
          throw new Error(`${name}: parameter "${pName}" is required`)
        }
      }
      return await runFn(args, context)
    }
  }
}

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
        const filePath = path.join(skillsDir, file)
        const stat = await fs.stat(filePath)
        
        if (stat.isDirectory()) {
          let mdFilePath = null
          const possibleFiles = ['skill.md', 'SKILL.md']
          for (const pf of possibleFiles) {
            try {
              const testPath = path.join(filePath, pf)
              const pfStat = await fs.stat(testPath)
              if (pfStat.isFile()) {
                mdFilePath = testPath
                break
              }
            } catch {}
          }
          if (mdFilePath) {
            try {
              const mdContent = await fs.readFile(mdFilePath, 'utf8')
              const SkillClass = parseMarkdownSkill(mdContent)
              if (SkillClass) {
                this.register(SkillClass)
                console.log(`  [skills] carregada (.md de pasta): ${SkillClass.actionName} (${file})`)
              }
            } catch (mdErr) {
              console.error(`  [skills] falha ao processar skill em pasta "${file}":`, mdErr.message)
            }
          }
        } else if (stat.isFile()) {
          if (file.endsWith('.js')) {
            const fileUrl = pathToFileURL(filePath).href + `?t=${Date.now()}`
            const { default: SkillClass } = await import(fileUrl)
            if (SkillClass && SkillClass.actionName) {
              this.register(SkillClass)
              console.log(`  [skills] carregada: ${SkillClass.actionName}`)
            }
          } else if (file.endsWith('.md')) {
            try {
              const mdContent = await fs.readFile(filePath, 'utf8')
              const SkillClass = parseMarkdownSkill(mdContent)
              if (SkillClass) {
                this.register(SkillClass)
                console.log(`  [skills] carregada (.md): ${SkillClass.actionName}`)
              }
            } catch (mdErr) {
              console.error(`  [skills] falha ao processar skill Markdown "${file}":`, mdErr.message)
            }
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
