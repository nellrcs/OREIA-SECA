// src/inputs/terminal.js
import readline from 'readline'
import { BaseInput } from './base-input.js'

export class TerminalInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._rl       = null
    this._typing   = false
  }

  async start() {
    this._rl = readline.createInterface({
      input:     process.stdin,
      output:    process.stdout,
      terminal:  true,
      prompt:    '\x1b[36m>\x1b[0m ',   // prompt colorido
    })

    console.log('\x1b[90m━━━ OREIASECA — terminal pronto. Ctrl+C para sair. ━━━\x1b[0m\n')
    this._rl.prompt()

    this._rl.on('line', async (line) => {
      const text = line.trim()
      if (!text) { this._rl.prompt(); return }

      this._clearTyping()

      await this._handler?.({
        source:   'terminal',
        userId:   'local',
        text,
        metadata: {},
      })
    })

    this._rl.on('close', () => {
      console.log('\n\x1b[90m[terminal] encerrado\x1b[0m')
    })
  }

  async stop() {
    this._rl?.close()
  }

  async send(userId, text) {
    this._clearTyping()
    // Quebra linha antes da resposta para não misturar com o prompt
    process.stdout.write('\r\x1b[K')
    // Prefixo visual para diferenciar resposta do agente
    const lines = text.split('\n')
    for (const line of lines) {
      console.log(`\x1b[90m│\x1b[0m ${line}`)
    }
    console.log()
    this._rl?.prompt()
  }

  async sendTyping(userId) {
    this._typing = true
    process.stdout.write('\r\x1b[K\x1b[90m│ digitando...\x1b[0m')
  }

  _clearTyping() {
    if (this._typing) {
      process.stdout.write('\r\x1b[K')
      this._typing = false
    }
  }
}
