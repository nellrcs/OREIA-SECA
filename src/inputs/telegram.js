// src/inputs/telegram.js
import { BaseInput } from './base-input.js'

const CHUNK_SIZE   = 4000   // Telegram aceita até 4096 chars por mensagem
const TYPING_MS    = 4000   // Reenvia "digitando" a cada N ms

export class TelegramInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._bot          = null
    this._typingTimers = new Map()   // userId → intervalId
  }

  async start() {
    // Importação lazy — não quebra se a lib não estiver instalada
    let TelegramBot
    try {
      const mod  = await import('node-telegram-bot-api')
      TelegramBot = mod.default ?? mod
    } catch {
      throw new Error(
        'node-telegram-bot-api não instalado.\n' +
        'Execute: npm install node-telegram-bot-api'
      )
    }

    this._bot = new TelegramBot(this.cfg.token, { polling: true })

    this._bot.on('message', async (msg) => {
      const userId = String(msg.from?.id)
      const chatId = msg.chat?.id
      const text   = msg.text?.trim()

      if (!text) return

      // Lista de usuários permitidos ([] = todos)
      if (this.cfg.allowedUsers?.length && !this.cfg.allowedUsers.includes(userId)) {
        await this._bot.sendMessage(chatId, '⛔ Acesso não autorizado.')
        return
      }

      await this._handler?.({
        source:   'telegram',
        userId,
        text,
        metadata: { chatId, username: msg.from?.username },
      })
    })

    this._bot.on('polling_error', (err) => {
      console.error('[telegram] polling error:', err.message)
    })

    console.log('[telegram] bot iniciado — aguardando mensagens')
  }

  async stop() {
    this._clearAllTyping()
    await this._bot?.stopPolling()
  }

  async send(userId, text) {
    this._stopTyping(userId)

    if (!this._bot) return

    // chatId === userId em chats privados
    const chatId = userId
    const chunks = this._chunk(text)

    for (const chunk of chunks) {
      await this._bot.sendMessage(chatId, chunk, {
        parse_mode: 'Markdown',
      }).catch(() =>
        // Fallback sem markdown se a formatação quebrar
        this._bot.sendMessage(chatId, chunk)
      )
    }
  }

  async sendTyping(userId) {
    if (!this._bot) return

    const chatId = userId

    // Envia imediatamente e agenda reenvio periódico
    // (Telegram mostra "digitando" por ~5s, então precisa renovar)
    const send = () => this._bot.sendChatAction(chatId, 'typing').catch(() => {})
    send()

    if (!this._typingTimers.has(userId)) {
      const id = setInterval(send, TYPING_MS)
      this._typingTimers.set(userId, id)
    }
  }

  _stopTyping(userId) {
    const id = this._typingTimers.get(userId)
    if (id) {
      clearInterval(id)
      this._typingTimers.delete(userId)
    }
  }

  _clearAllTyping() {
    for (const id of this._typingTimers.values()) clearInterval(id)
    this._typingTimers.clear()
  }

  _chunk(text) {
    if (text.length <= CHUNK_SIZE) return [text]
    const chunks = []
    let i = 0
    while (i < text.length) {
      chunks.push(text.slice(i, i + CHUNK_SIZE))
      i += CHUNK_SIZE
    }
    return chunks
  }
}
