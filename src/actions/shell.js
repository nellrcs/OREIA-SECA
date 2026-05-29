// src/actions/shell.js
import { exec }      from 'child_process'
import { promisify } from 'util'
import path          from 'path'
import fs            from 'fs/promises'
import { BaseAction } from './base-action.js'

const execAsync = promisify(exec)

const BLOCKED = [
  /rm\s+-rf\s+\//,
  /mkfs/,
  /dd\s+if=/,
  /:\(\)\{.*\}/,
  /curl.*\|\s*(ba)?sh/,
  /wget.*\|\s*(ba)?sh/,
]

export class ShellAction extends BaseAction {
  static actionName = 'shell'
  static description = 'Executes a shell command in the operating system. Use with caution. CRITICAL: For ALL Docker operations, you MUST use "skill_docker". For ALL Git operations (cloning, pulling, status), you MUST use "skill_git". For downloading files from URLs, you MUST use "skill_download". For web browsing/scraping/automations, you MUST use "skill_browser". Never run raw command-line tools for these tasks.'
  static params = {
    command: { type: 'string', description: 'The shell command to run (e.g., "npm install express").', required: true },
    cwd: { type: 'string', description: 'Optional subdirectory relative to the workspace directory to run the command in.', required: false }
  }

  validate({ command }) {
    if (!command) throw new Error('shell: param "command" obrigatório')
    for (const pattern of BLOCKED) {
      if (pattern.test(command))
        throw new Error(`shell: comando bloqueado — ${command}`)
    }
  }

  async run({ command, cwd }, context) {
    this.validate({ command })

    // No modo direto (sem taskId), executa no diretório de trabalho atual
    const workdir = context.taskId
      ? path.resolve('workspace', context.taskId)
      : process.cwd()

    // Garantir que a pasta workdir do workspace exista antes de rodar qualquer comando nela
    await fs.mkdir(workdir, { recursive: true })

    try {
      const { stdout, stderr } = await execAsync(command, {
        cwd:     cwd ? path.join(workdir, cwd) : workdir,
        timeout: 30_000,
      })
      const output = [stdout, stderr].filter(Boolean).join('\n').trim()
      return output ? `$ ${command}\n${output}` : `$ ${command}\n(sem output)`
    } catch (err) {
      const stdout = err.stdout || ''
      const stderr = err.stderr || err.message
      const output = [stdout, stderr].filter(Boolean).join('\n').trim()
      throw new Error(`Command failed: ${command}\nOutput:\n${output}`)
    }
  }
}
