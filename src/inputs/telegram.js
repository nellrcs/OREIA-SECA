// src/inputs/telegram.js
import { BaseInput } from './base-input.js'

const CHUNK_SIZE = 4000   // Telegram max ~4096 chars
const TYPING_MS  = 4000   // Reenvio de "digitando" a cada N ms

// ─── Teclado de comandos rápidos ────────────────────────────────────────────

function makeMainKeyboard() {
  return {
    keyboard: [
      [{ text: '📊 /status' }, { text: '📋 /queue' }],
      [{ text: '🛠 /skills' }, { text: '📁 /resumos' }],
      [{ text: '⏳ /pending' }, { text: '📝 /resumir' }],
    ],
    resize_keyboard: true,
    one_time_keyboard: false,
    input_field_placeholder: 'Digite uma tarefa ou comando...',
  }
}

export class TelegramInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._bot          = null
    this._typingTimers = new Map()
    this._knownChats   = new Set()
    // Map de taskId → { chatId, messageId } para edição de msgs de aprovação
    this._approvalMsgs = new Map()
  }

  // ─── Start ─────────────────────────────────────────────────────────────────

  async start() {
    let TelegramBot
    try {
      process.env.NTBA_FIX_350 = '1'
      const mod  = await import('node-telegram-bot-api')
      TelegramBot = mod.default ?? mod
    } catch {
      throw new Error(
        'node-telegram-bot-api não instalado.\n' +
        'Execute: npm install node-telegram-bot-api'
      )
    }

    this._bot = new TelegramBot(this.cfg.token, { polling: true })

    // Pré-preenche chats conhecidos
    for (const uid of (this.cfg.allowedUsers ?? [])) {
      this._knownChats.add(String(uid))
    }

    // ─── /start ──────────────────────────────────────────────────────────
    this._bot.onText(/\/start/, async (msg) => {
      const chatId = String(msg.chat.id)
      this._knownChats.add(chatId)
      try {
        await this._bot.sendMessage(chatId,
          '🤖 *OREIA.SECA* — Agente de IA Online!\n\n' +
          'Envie uma tarefa em linguagem natural ou toque num comando abaixo.',
          { parse_mode: 'Markdown', reply_markup: makeMainKeyboard() }
        )
      } catch {
        await this._bot.sendMessage(chatId,
          '🤖 OREIA.SECA — Online! Envie uma tarefa ou use os comandos.',
          { reply_markup: makeMainKeyboard() }
        ).catch(err => console.error('[telegram] erro ao enviar /start:', err.message))
      }
    })

    // ─── Mensagens de texto ──────────────────────────────────────────────
    this._bot.on('message', async (msg) => {
      try {
        if (msg.text?.startsWith('/start')) return

        const chatId = String(msg.chat?.id)
        // Normaliza botões do teclado: "📊 /status" → "/status"
        const rawText = msg.text?.trim() ?? ''
        const text    = rawText.replace(/^[^\w/]*\//, '/').trim()

        if (!text) return

        this._knownChats.add(chatId)
        console.log(`[telegram] mensagem de ${chatId}: ${text.slice(0, 60)}`)

        // ACL
        if (this.cfg.allowedUsers?.length &&
            !this.cfg.allowedUsers.includes(String(msg.from?.id))) {
          console.log(`[telegram] acesso negado para userId=${msg.from?.id}`)
          await this._bot.sendMessage(msg.chat.id, '⛔ Acesso não autorizado.')
          return
        }

        if (!this._handler) return

        await this._handler({
          source:   'telegram',
          userId:   chatId,
          text,
          metadata: { chatId: msg.chat.id, username: msg.from?.username },
        })
      } catch (err) {
        console.error('[telegram] erro ao processar mensagem:', err)
        try { await this._bot.sendMessage(msg.chat?.id, `❌ Erro interno: ${err.message}`) } catch {}
      }
    })

    // ─── Callback de botões inline ───────────────────────────────────────
    this._bot.on('callback_query', async (query) => {
      try {
        const chatId = String(query.message?.chat?.id)
        const data   = query.data ?? ''
        const msgId  = query.message?.message_id

        // ACL
        if (this.cfg.allowedUsers?.length &&
            !this.cfg.allowedUsers.includes(String(query.from?.id))) {
          await this._bot.answerCallbackQuery(query.id, { text: '⛔ Não autorizado.' })
          return
        }

        // Remove spinner do botão imediatamente
        await this._bot.answerCallbackQuery(query.id).catch(() => {})

        if (this._handler && data) {
          await this._handler({
            source:   'telegram',
            userId:   chatId,
            text:     data,    // ex: "/approve task-abc123"
            metadata: { chatId: query.message?.chat?.id, callbackMsgId: msgId },
          })
        }
      } catch (err) {
        console.error('[telegram] erro no callback_query:', err.message)
      }
    })

    // ─── Polling errors ──────────────────────────────────────────────────
    this._bot.on('polling_error', (err) => {
      if (err.message?.includes('ETIMEDOUT')) return
      if (err.message?.includes('409 Conflict')) {
        console.warn('[telegram] ⚠ conflito de polling — outra instância rodando?')
        return
      }
      console.error('[telegram] polling error:', err.message)
    })

    // ─── Boot ────────────────────────────────────────────────────────────
    try {
      const me = await this._bot.getMe()
      console.log(`[telegram] bot: @${me.username} (id=${me.id})`)

      for (const chatId of this._knownChats) {
        await this._bot.sendMessage(chatId,
          '🟢 *OREIA.SECA online* — reconectado!',
          { parse_mode: 'Markdown', reply_markup: makeMainKeyboard() }
        ).catch(err =>
          console.error(`[telegram] falha ao notificar ${chatId}: ${err.message}`)
        )
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

  // ─── send: texto com fallback Markdown → plain ───────────────────────────

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
        // Tenta Markdown → plain text (fallback em cascata)
        let sent = false
        for (const parse_mode of ['Markdown', undefined]) {
          try {
            await this._bot.sendMessage(chatId, chunk, parse_mode ? { parse_mode } : {})
            sent = true
            break
          } catch (err) {
            if (parse_mode === undefined) {
              console.error(`[telegram] falha ao enviar para ${chatId}: ${err.message}`)
            }
          }
        }
        if (!sent) {
          try { await this._bot.sendMessage(chatId, chunk.slice(0, 4000)) } catch {}
        }
      }
    }
  }

  // ─── sendButtons: mensagem com teclado inline ────────────────────────────

  /**
   * Envia mensagem com botões inline.
   * @param {string} userId
   * @param {string} text  — suporta Markdown simples
   * @param {Array<Array<{text:string, callback_data:string}>>} buttons — grade de botões
   * @returns {Promise<object|null>}  mensagem enviada (use para editMessage())
   */
  async sendButtons(userId, text, buttons) {
    if (!this._bot) return null
    const chatId = String(userId)
    try {
      return await this._bot.sendMessage(chatId, text, {
        parse_mode:   'Markdown',
        reply_markup: { inline_keyboard: buttons },
      })
    } catch {
      try {
        return await this._bot.sendMessage(chatId, text, {
          reply_markup: { inline_keyboard: buttons },
        })
      } catch (err) {
        console.error(`[telegram] sendButtons falhou para ${chatId}: ${err.message}`)
        return null
      }
    }
  }

  // ─── editMessage: edita mensagem existente ───────────────────────────────

  /**
   * Edita o texto (e botões) de uma mensagem já enviada.
   * Silencia o erro "message is not modified" do Telegram.
   */
  async editMessage(chatId, messageId, newText, buttons = null) {
    if (!this._bot || !messageId) return
    try {
      const opts = { parse_mode: 'Markdown' }
      if (buttons) opts.reply_markup = { inline_keyboard: buttons }
      await this._bot.editMessageText(newText, {
        chat_id:    String(chatId),
        message_id: messageId,
        ...opts,
      })
    } catch (err) {
      if (!err.message?.includes('not modified')) {
        console.warn(`[telegram] editMessage falhou: ${err.message}`)
      }
    }
  }

  // ─── Typing indicator ─────────────────────────────────────────────────────

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

  // ─── Internos ─────────────────────────────────────────────────────────────

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

  /**
   * Divide texto longo em chunks sem quebrar no meio de palavras markdown.
   * Prefere quebrar em newlines quando possível.
   */
  _chunk(text) {
    if (text.length <= CHUNK_SIZE) return [text]
    const chunks = []
    let i = 0
    while (i < text.length) {
      let end = i + CHUNK_SIZE
      if (end < text.length) {
        const nl = text.lastIndexOf('\n', end)
        if (nl > i + CHUNK_SIZE / 2) end = nl + 1
      }
      chunks.push(text.slice(i, end))
      i = end
    }
    return chunks
  }
}
