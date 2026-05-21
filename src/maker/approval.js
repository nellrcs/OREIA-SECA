// src/maker/approval.js

/**
 * Gerencia aprovações cross-canal.
 *
 * Fluxo:
 *   1. REST (ou cron) envia tarefa
 *   2. Maker planeja e chama requestApproval()
 *   3. ApprovalBroker envia a pergunta para terminal + telegram
 *   4. Usuário responde /approve <id> ou /reject <id> em qualquer canal
 *   5. A Promise resolve e o Maker executa (ou cancela)
 */
export class ApprovalBroker {
  constructor() {
    // taskId → { resolve, reject, timeout, question }
    this._pending = new Map()
  }

  /**
   * Envia pedido de aprovação para múltiplos inputs e aguarda resposta.
   *
   * @param {string}   taskId   — ID da tarefa
   * @param {string}   question — texto com o plano para aprovar
   * @param {BaseInput[]} inputs — canais para broadcast (terminal, telegram, etc.)
   * @param {number}   timeoutMs — tempo máximo de espera (padrão: 10 minutos)
   * @returns {Promise<boolean>} — true se aprovado, false se rejeitado
   */
  request(taskId, question, inputs, timeoutMs = 600_000) {
    return new Promise((resolve, reject) => {
      // Timeout automático — rejeita se ninguém responder
      const timeout = setTimeout(() => {
        if (this._pending.has(taskId)) {
          this._pending.delete(taskId)
          console.warn(`[approval] ${taskId} expirou sem resposta`)
          resolve(false)
        }
      }, timeoutMs)

      this._pending.set(taskId, { resolve, timeout, question })

      // Broadcast para todos os inputs
      const msg =
        `🔔 *Aprovação necessária*\n\n` +
        `${question}\n\n` +
        `Responda:\n` +
        `  /approve ${taskId}\n` +
        `  /reject ${taskId}`

      for (const input of inputs) {
        // Envia para todos os userIds conhecidos (ou broadcast)
        input.send?.('broadcast', msg).catch(() => {})
      }

      console.log(`[approval] aguardando aprovação para ${taskId}`)
    })
  }

  /**
   * Processa uma resposta de aprovação.
   * Chamado pelo handleCommand do Maker quando recebe /approve ou /reject.
   *
   * @param {string}  taskId  — ID da tarefa
   * @param {boolean} approved — true para aprovar, false para rejeitar
   * @returns {boolean} — true se havia uma aprovação pendente
   */
  respond(taskId, approved) {
    const entry = this._pending.get(taskId)
    if (!entry) return false

    clearTimeout(entry.timeout)
    this._pending.delete(taskId)
    entry.resolve(approved)

    console.log(`[approval] ${taskId} ${approved ? 'aprovado ✓' : 'rejeitado ✖'}`)
    return true
  }

  /**
   * Verifica se há aprovação pendente para um taskId.
   */
  isPending(taskId) {
    return this._pending.has(taskId)
  }

  /**
   * Lista todas as aprovações pendentes.
   */
  listPending() {
    return [...this._pending.entries()].map(([id, { question }]) => ({
      id,
      question: question?.slice(0, 80),
    }))
  }
}
