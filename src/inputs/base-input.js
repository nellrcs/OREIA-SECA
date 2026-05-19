// src/inputs/base-input.js

export class BaseInput {
  constructor(cfg = {}) {
    this.cfg      = cfg
    this._handler = null
  }

  // Registra o callback que recebe mensagens padronizadas
  // callback({ source, userId, text, metadata })
  onMessage(callback) {
    this._handler = callback
    return this
  }

  // Inicia o canal (abre readline, polling, websocket...)
  async start() {
    throw new Error(`${this.constructor.name}: start() não implementado`)
  }

  // Para de receber novas mensagens
  async stop() {}

  // Envia texto de volta ao usuário
  async send(userId, text) {
    throw new Error(`${this.constructor.name}: send() não implementado`)
  }

  // Indica que o sistema está processando (digitando...)
  async sendTyping(userId) {}

  // Nome do canal para logs
  get name() {
    return this.constructor.name.replace('Input', '').toLowerCase()
  }
}
