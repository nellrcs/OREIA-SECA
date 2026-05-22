// src/actions/filesystem.js
import fs   from 'fs/promises'
import path from 'path'
import { BaseAction } from './base-action.js'

export class FileWriteAction extends BaseAction {
  static actionName = 'file_write'
  static description = 'Writes text content to a file. Access is strictly sandboxed inside the task workspace directory.'
  static params = {
    path: { type: 'string', description: 'The relative path of the file to write (e.g., "src/main.js"). Cannot use ".." to escape the workspace.', required: true },
    content: { type: 'string', description: 'The raw text content to write into the file.', required: true }
  }

  validate({ path: filePath, content }, workdir) {
    if (!filePath)           throw new Error('file_write: param "path" obrigatório')
    if (content === undefined) throw new Error('file_write: param "content" obrigatório')
    if (filePath.includes('..')) throw new Error(`file_write: path traversal bloqueado — ${filePath}`)
    
    const fullPath = path.resolve(workdir, filePath)
    if (!fullPath.startsWith(workdir)) {
      throw new Error(`file_write: path traversal bloqueado — tentativa de gravar fora do workspace`)
    }
  }

  async run({ path: filePath, content }, context) {
    const workdir  = context.taskId
      ? path.resolve('workspace', context.taskId)
      : path.resolve('workspace')

    this.validate({ path: filePath, content }, workdir)
    const fullPath = path.join(workdir, filePath)

    await fs.mkdir(path.dirname(fullPath), { recursive: true })
    await fs.writeFile(fullPath, content, 'utf-8')

    const lines = content.split('\n').length
    return `arquivo criado: ${filePath} (${lines} linhas)`
  }
}

export class FileReadAction extends BaseAction {
  static actionName = 'file_read'
  static description = 'Reads text content from a file inside the task workspace.'
  static params = {
    path: { type: 'string', description: 'The relative path of the file to read (e.g., "src/main.js"). Cannot use ".." to escape the workspace.', required: true },
    lines: { type: 'string', description: 'Optional line range to read (e.g., "1-20" or "50-100").', required: false }
  }

  validate({ path: filePath }, workdir) {
    if (!filePath)               throw new Error('file_read: param "path" obrigatório')
    if (filePath.includes('..')) throw new Error('file_read: path traversal bloqueado')
    
    const fullPath = path.resolve(workdir, filePath)
    if (!fullPath.startsWith(workdir)) {
      throw new Error('file_read: path traversal bloqueado — tentativa de ler fora do workspace')
    }
  }

  async run({ path: filePath, lines }, context) {
    const workdir  = context.taskId
      ? path.resolve('workspace', context.taskId)
      : path.resolve('workspace')

    this.validate({ path: filePath }, workdir)
    const fullPath = path.join(workdir, filePath)

    let content = await fs.readFile(fullPath, 'utf-8')

    if (lines) {
      const [start, end] = lines.split('-').map(Number)
      content = content.split('\n').slice(start - 1, end).join('\n')
    }

    return `=== ${filePath} ===\n${content}`
  }
}
