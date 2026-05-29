// src/inputs/telegram.js
import { BaseInput } from './base-input.js'

const CHUNK_SIZE   = 4000   // Telegram aceita até 4096 chars por mensagem
const TYPING_MS    = 4000   // Reenvia "digitando" a cada N ms

export class TelegramInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._bot          = null
    this._typingTimers = new Map()
    this._statusMsgs   = new Map()
    this._knownChats   = new Set()
  }

  async start() {
    let TelegramBot
    try {
      process.env.NTBA_FIX_350 = '1'; // Silencia deprecation warnings do node-telegram-bot-api
      const mod  = await import('node-telegram-bot-api')
      TelegramBot = mod.default ?? mod
    } catch {
      throw new Error(
        'node-telegram-bot-api não instalado.\n' +
        'Execute: npm install node-telegram-bot-api'
      )
    }

    this._bot = new TelegramBot(this.cfg.token, { polling: true })

    // Pré-preenche chats conhecidos a partir dos allowedUsers
    for (const uid of (this.cfg.allowedUsers ?? [])) {
      this._knownChats.add(String(uid))
    }

    // ─── /start — boas-vindas ───────────────────────────────────────────
    this._bot.onText(/\/start/, async (msg) => {
      const chatId = msg.chat.id
      this._knownChats.add(String(chatId))
      try {
        await this._bot.sendMessage(chatId,
          '🤖 OREIASECA — Agente de IA\n\n' +
          'Envie uma mensagem em linguagem natural.\n\n' +
          'Comandos: /status /tasks /queue /retry /approve /reject /pending'
        )
      } catch (err) {
        console.error('[telegram] erro ao enviar /start:', err.message)
      }
    })

    // ─── Mensagens de texto ─────────────────────────────────────────────
    this._bot.on('message', async (msg) => {
      try {
        if (msg.text?.startsWith('/start')) return

        const chatId = String(msg.chat?.id)
        const text   = msg.text?.trim()

        if (!text) return

        this._knownChats.add(chatId)

        console.log(`[telegram] mensagem recebida de ${chatId}: ${text.slice(0, 60)}`)

        // Lista de usuários permitidos ([] = todos)
        if (this.cfg.allowedUsers?.length && !this.cfg.allowedUsers.includes(String(msg.from?.id))) {
          console.log(`[telegram] acesso negado para userId=${msg.from?.id}`)
          await this._bot.sendMessage(msg.chat.id, '⛔ Acesso não autorizado.')
          return
        }

        if (!this._handler) {
          console.error('[telegram] _handler não definido — mensagem ignorada')
          return
        }

        await this._handler({
          source:   'telegram',
          userId:   chatId,
          text,
          metadata: { chatId: msg.chat.id, username: msg.from?.username },
        })
      } catch (err) {
        // Captura QUALQUER erro para não matar o processo
        console.error('[telegram] erro ao processar mensagem:', err)
        try {
          await this._bot.sendMessage(msg.chat?.id, `❌ Erro interno: ${err.message}`)
        } catch {}
      }
    })

    this._bot.on('polling_error', (err) => {
      if (err.message?.includes('ETIMEDOUT')) return
      if (err.message?.includes('409 Conflict')) {
        console.warn('[telegram] ⚠ conflito de polling — outra instância do bot está rodando?')
        return
      }
      console.error('[telegram] polling error:', err.message)
    })

    // ─── Verificação de boot ────────────────────────────────────────────
    try {
      const me = await this._bot.getMe()
      console.log(`[telegram] bot verificado: @${me.username} (id=${me.id})`)

      // Envia mensagem proativa para confirmar que o envio funciona
      for (const chatId of this._knownChats) {
        await this._bot.sendMessage(chatId, '🟢 OREIASECA online — bot conectado!').catch(err => {
          console.error(`[telegram] falha ao enviar proativa para ${chatId}: ${err.message}`)
        })
      }
    } catch (err) {
      console.error('[telegram] falha ao verificar bot (getMe):', err.message)
    }

    console.log('[telegram] bot iniciado — aguardando mensagens')
  }

  async stop() {
    this._clearAllTyping()
    if (this._bot) {
      await this._bot.stopPolling().catch(() => {})
      this._bot = null
    }
  }

  // ─── Enviar mensagem ────────────────────────────────────────────────────

  async send(userId, text) {
    this._stopTyping(userId)
    if (!this._bot) return

    const targets = userId === 'broadcast'
      ? [...this._knownChats]
      : [String(userId)]

    if (!targets.length) {
      console.warn('[telegram] send sem destinatários')
      return
    }

    const chunks = this._chunk(text)

    for (const chatId of targets) {
      for (const chunk of chunks) {
        try {
          await this._bot.sendMessage(chatId, chunk, { parse_mode: 'Markdown' })
        } catch {
          try {
            await this._bot.sendMessage(chatId, chunk)
          } catch (err) {
            console.error(`[telegram] falha ao enviar para ${chatId}: ${err.message}`)
          }
        }
      }
    }
  }

  // ─── Typing indicator ──────────────────────────────────────────────────

  async sendTyping(userId) {
    if (!this._bot) return
    const chatId = userId

    const doSend = () => this._bot?.sendChatAction(chatId, 'typing').catch(() => {})
    doSend()

    if (!this._typingTimers.has(userId)) {
      const id = setInterval(doSend, TYPING_MS)
      this._typingTimers.set(userId, id)
    }
  }

  // ─── Internos ──────────────────────────────────────────────────────────

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
