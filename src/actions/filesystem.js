// src/actions/filesystem.js
import fs   from 'fs/promises'
import path from 'path'
import { BaseAction } from './base-action.js'

export class FileWriteAction extends BaseAction {
  static actionName = 'file_write'

  validate({ path: filePath, content }) {
    if (!filePath)           throw new Error('file_write: param "path" obrigatório')
    if (content === undefined) throw new Error('file_write: param "content" obrigatório')
    if (filePath.includes('..')) throw new Error(`file_write: path traversal bloqueado — ${filePath}`)
  }

  async run({ path: filePath, content }, context) {
    this.validate({ path: filePath, content })

    const workdir  = path.resolve('workspace', context.taskId)
    const fullPath = path.join(workdir, filePath)

    await fs.mkdir(path.dirname(fullPath), { recursive: true })
    await fs.writeFile(fullPath, content, 'utf-8')

    const lines = content.split('\n').length
    return `arquivo criado: ${filePath} (${lines} linhas)`
  }
}

export class FileReadAction extends BaseAction {
  static actionName = 'file_read'

  validate({ path: filePath }) {
    if (!filePath)               throw new Error('file_read: param "path" obrigatório')
    if (filePath.includes('..')) throw new Error('file_read: path traversal bloqueado')
  }

  async run({ path: filePath, lines }, context) {
    this.validate({ path: filePath })

    const workdir  = path.resolve('workspace', context.taskId)
    const fullPath = path.join(workdir, filePath)

    let content = await fs.readFile(fullPath, 'utf-8')

    if (lines) {
      const [start, end] = lines.split('-').map(Number)
      content = content.split('\n').slice(start - 1, end).join('\n')
    }

    return `=== ${filePath} ===\n${content}`
  }
}
