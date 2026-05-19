// src/maker/task-queue.js

export class TaskQueue {
  constructor({ concurrency = 1, timeout = 0 } = {}) {
    this.concurrency = concurrency
    this.timeout     = timeout
    this.running     = 0
    this.queue       = []
    this.stats       = { completed: 0, failed: 0, queued: 0 }
    this._active     = new Set()   // Promises das tarefas em execução
  }

  add(fn, label = '') {
    if (this.running < this.concurrency) {
      return this._run(fn, label)
    }
    this.stats.queued++
    console.log(`[queue] "${label}" enfileirada (posição ${this.queue.length + 1})`)
    return new Promise((resolve, reject) => {
      this.queue.push({ fn, resolve, reject, label })
    })
  }

  async _run(fn, label) {
    this.running++
    console.log(`[queue] iniciando "${label}" | rodando: ${this.running}/${this.concurrency} | fila: ${this.queue.length}`)

    const p = (async () => {
      try {
        const result = this.timeout > 0
          ? await Promise.race([fn(), this._timeoutPromise(label)])
          : await fn()
        this.stats.completed++
        return result
      } catch (err) {
        this.stats.failed++
        throw err
      } finally {
        this.running--
        this._active.delete(p)
        this._next()
      }
    })()

    this._active.add(p)
    return p
  }

  _timeoutPromise(label) {
    return new Promise((_, reject) =>
      setTimeout(() => reject(new Error(`[queue] timeout: "${label}" excedeu ${this.timeout}ms`)), this.timeout)
    )
  }

  _next() {
    if (this.queue.length === 0 || this.running >= this.concurrency) return
    const { fn, resolve, reject, label } = this.queue.shift()
    this._run(fn, label).then(resolve).catch(reject)
  }

  get status() {
    return {
      running:     this.running,
      queued:      this.queue.length,
      concurrency: this.concurrency,
      stats:       { ...this.stats },
    }
  }

  async drain(timeoutMs = 10_000) {
    if (this.running === 0 && this.queue.length === 0) return
    console.log(`[queue] drain — ${this.running} rodando, ${this.queue.length} na fila`)
    await Promise.race([
      Promise.allSettled([...this._active]),
      new Promise(r => setTimeout(r, timeoutMs)),
    ])
  }
}
