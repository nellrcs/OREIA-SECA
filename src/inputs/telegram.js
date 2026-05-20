// src/inputs/telegram.js
import { BaseInput } from './base-input.js'

const CHUNK_SIZE   = 4000   // Telegram aceita até 4096 chars por mensagem
const TYPING_MS    = 4000   // Reenvia "digitando" a cada N ms

// Caracteres que precisam de escape no MarkdownV2 do Telegram
const MD_V2_ESCAPE = /([_*\[\]()~`>#+\-=|{}.!\\])/g

function escapeMarkdownV2(text) {
  // Preserva blocos de código — não escapar dentro deles
  const parts = text.split(/(```[\s\S]*?```|`[^`]+`)/g)
  return parts.map((part, i) => {
    // Índices ímpares são blocos de código capturados, mantém como estão
    if (i % 2 === 1) return part
    return part.replace(MD_V2_ESCAPE, '\\$1')
  }).join('')
}

export class TelegramInput extends BaseInput {
  constructor(cfg = {}) {
    super(cfg)
    this._bot          = null
    this._typingTimers = new Map()   // userId → intervalId
    this._statusMsgs   = new Map()   // userId → messageId (para editar)
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

    // ─── /start — boas-vindas ───────────────────────────────────────────
    this._bot.onText(/\/start/, async (msg) => {
      const chatId = msg.chat.id
      await this._bot.sendMessage(chatId,
        '🤖 *OREIASECA* — Agente de IA\n\n' +
        'Envie uma mensagem em linguagem natural e eu vou:\n' +
        '• Responder perguntas técnicas\n' +
        '• Executar comandos no servidor\n' +
        '• Planejar e executar tarefas complexas\n\n' +
        'Comandos: /status /tasks /queue /retry',
        { parse_mode: 'Markdown' }
      )
    })

    // ─── Mensagens de texto ─────────────────────────────────────────────
    this._bot.on('message', async (msg) => {
      // Ignora se for um comando /start (já tratado acima)
      if (msg.text?.startsWith('/start')) return

      const userId = String(msg.chat?.id)
      const text   = msg.text?.trim()

      if (!text) return

      // Lista de usuários permitidos ([] = todos)
      if (this.cfg.allowedUsers?.length && !this.cfg.allowedUsers.includes(String(msg.from?.id))) {
        await this._bot.sendMessage(msg.chat.id, '⛔ Acesso não autorizado.')
        return
      }

      await this._handler?.({
        source:   'telegram',
        userId,
        text,
        metadata: { chatId: msg.chat.id, username: msg.from?.username },
      })
    })

    // ─── Callback queries (futuro: botões inline) ───────────────────────
    this._bot.on('callback_query', async (query) => {
      await this._bot.answerCallbackQuery(query.id)
    })

    this._bot.on('polling_error', (err) => {
      // Ignora erros de timeout (comuns e não-críticos)
      if (err.code === 'ETELEGRAM' && err.message?.includes('ETIMEDOUT')) return
      console.error('[telegram] polling error:', err.message)
    })

    console.log('[telegram] bot iniciado — aguardando mensagens')
  }

  async stop() {
    this._clearAllTyping()
    await this._bot?.stopPolling()
  }

  // ─── Enviar mensagem ────────────────────────────────────────────────────

  async send(userId, text) {
    this._stopTyping(userId)
    if (!this._bot) return

    const chatId = userId
    const chunks = this._chunk(text)

    for (const chunk of chunks) {
      // Tenta Markdown primeiro, fallback para texto puro
      await this._bot.sendMessage(chatId, chunk, {
        parse_mode: 'Markdown',
      }).catch(() =>
        this._bot.sendMessage(chatId, chunk).catch(() => {})
      )
    }

    // Limpa a mensagem de status se houver
    this._clearStatusMsg(userId)
  }

  // ─── Typing indicator ──────────────────────────────────────────────────

  async sendTyping(userId) {
    if (!this._bot) return

    const chatId = userId

    // Envia a ação de digitando imediatamente
    const sendAction = () => this._bot.sendChatAction(chatId, 'typing').catch(() => {})
    sendAction()

    // Renova periodicamente (Telegram mostra "digitando" por ~5s)
    if (!this._typingTimers.has(userId)) {
      const id = setInterval(sendAction, TYPING_MS)
      this._typingTimers.set(userId, id)
    }
  }

  // ─── Mensagem de progresso editável ────────────────────────────────────

  async sendProgress(userId, text) {
    if (!this._bot) return
    const chatId = userId

    const existing = this._statusMsgs.get(userId)

    if (existing) {
      // Edita a mensagem existente em vez de enviar uma nova
      await this._bot.editMessageText(text, {
        chat_id: chatId,
        message_id: existing,
      }).catch(() => {})
    } else {
      // Cria a primeira mensagem de progresso
      try {
        const msg = await this._bot.sendMessage(chatId, text)
        this._statusMsgs.set(userId, msg.message_id)
      } catch {}
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

  _clearStatusMsg(userId) {
    this._statusMsgs.delete(userId)
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
