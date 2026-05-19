// src/storage/task-store.js
import fs   from 'fs/promises'
import path from 'path'

const TASKS_DIR = path.resolve('tasks')

// Lock simples por taskId — evita escrita concorrente
const locks = new Map()

async function withLock(taskId, fn) {
  while (locks.get(taskId)) {
    await new Promise(r => setTimeout(r, 20))
  }
  locks.set(taskId, true)
  try {
    return await fn()
  } finally {
    locks.delete(taskId)
  }
}

class TaskStore {
  async _ensureDir() {
    await fs.mkdir(TASKS_DIR, { recursive: true })
  }

  _filePath(taskId) {
    return path.join(TASKS_DIR, `${taskId}.json`)
  }

  // ─── Salva (cria ou atualiza) ──────────────────────────────────────────────

  async save(task) {
    await this._ensureDir()

    return withLock(task.id, async () => {
      task.updatedAt = new Date().toISOString()
      const filePath = this._filePath(task.id)
      await fs.writeFile(filePath, JSON.stringify(task, null, 2), 'utf-8')
      return task
    })
  }

  // ─── Carrega por id ────────────────────────────────────────────────────────

  async load(taskId) {
    try {
      const raw = await fs.readFile(this._filePath(taskId), 'utf-8')
      return JSON.parse(raw)
    } catch (err) {
      if (err.code === 'ENOENT') return null
      throw err
    }
  }

  // ─── Lista todas as tarefas de um usuário ──────────────────────────────────

  async listByUser(userId) {
    await this._ensureDir()

    const files = await fs.readdir(TASKS_DIR)
    const tasks = []

    for (const file of files) {
      if (!file.endsWith('.json')) continue
      try {
        const raw  = await fs.readFile(path.join(TASKS_DIR, file), 'utf-8')
        const task = JSON.parse(raw)
        if (task.userId === userId) tasks.push(task)
      } catch {
        // arquivo corrompido — ignora
      }
    }

    // Mais recente primeiro
    return tasks.sort((a, b) =>
      new Date(b.createdAt) - new Date(a.createdAt)
    )
  }

  // ─── Lista todas as tarefas com um status específico ──────────────────────

  async listByStatus(status) {
    await this._ensureDir()

    const files = await fs.readdir(TASKS_DIR)
    const tasks = []

    for (const file of files) {
      if (!file.endsWith('.json')) continue
      try {
        const raw  = await fs.readFile(path.join(TASKS_DIR, file), 'utf-8')
        const task = JSON.parse(raw)
        if (task.status === status) tasks.push(task)
      } catch {}
    }

    return tasks
  }

  // ─── Atualiza um campo específico sem reescrever tudo ─────────────────────

  async patch(taskId, fields) {
    return withLock(taskId, async () => {
      const task = await this.load(taskId)
      if (!task) throw new Error(`task não encontrada: ${taskId}`)
      Object.assign(task, fields, { updatedAt: new Date().toISOString() })
      await fs.writeFile(this._filePath(taskId), JSON.stringify(task, null, 2), 'utf-8')
      return task
    })
  }

  // ─── Remove uma tarefa ─────────────────────────────────────────────────────

  async remove(taskId) {
    try {
      await fs.unlink(this._filePath(taskId))
    } catch (err) {
      if (err.code !== 'ENOENT') throw err
    }
  }
}

export const taskStore = new TaskStore()
