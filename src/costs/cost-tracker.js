// src/costs/cost-tracker.js
import { calcCost, formatUSD } from './pricing.js'

// Limiares de alerta (fração do budget)
const WARN_THRESHOLD = 0.80   // avisa ao usuário ao atingir 80%
const STOP_THRESHOLD = 1.00   // bloqueia ao atingir 100%

export class CostTracker {
  /**
   * @param {object} opts
   * @param {string} opts.modelName  — nome do modelo (para lookup de preço)
   * @param {number|null} opts.budget — limite em USD, ou null para monitorar sem bloquear
   * @param {Function} opts.onWarn   — callback({ spent, budget, percent }) ao atingir 80%
   * @param {Function} opts.onStop   — callback({ spent, budget }) ao atingir 100%
   */
  constructor({ modelName, budget = null, onWarn = null, onStop = null }) {
    this.modelName = modelName
    this.budget    = budget     // null = sem limite (só monitora)
    this.onWarn    = onWarn
    this.onStop    = onStop

    this.spent     = 0          // total gasto até agora (USD)
    this.calls     = []         // histórico de chamadas
    this.warned    = false      // evita disparar onWarn múltiplas vezes
  }

  // ─── Registra o custo de uma chamada ao modelo ────────────────────────────

  track(usage, label = '') {
    const cost = calcCost(usage, this.modelName)
    this.spent += cost

    this.calls.push({
      label,
      promptTokens:     usage.prompt_tokens,
      completionTokens: usage.completion_tokens,
      cost,
      cumulative: this.spent,
      at: new Date().toISOString(),
    })

    console.log(
      `[cost] ${label || 'chamada'} — ` +
      `${usage.prompt_tokens}in + ${usage.completion_tokens}out = ${formatUSD(cost)}` +
      (this.budget ? ` | total: ${formatUSD(this.spent)} / ${formatUSD(this.budget)}` : ` | total: ${formatUSD(this.spent)}`)
    )

    this._checkThresholds()
    return cost
  }

  // ─── Verifica limiares e dispara callbacks ────────────────────────────────

  _checkThresholds() {
    if (!this.budget) return

    const percent = this.spent / this.budget

    // Alerta de 80%
    if (!this.warned && percent >= WARN_THRESHOLD) {
      this.warned = true
      this.onWarn?.({
        spent:   this.spent,
        budget:  this.budget,
        percent: Math.round(percent * 100),
        remaining: this.budget - this.spent,
      })
    }
  }

  // ─── Verifica ANTES de uma chamada se ainda há budget ────────────────────
  // Chame antes de iniciar cada fase — lança BudgetExceededError se estourou

  assertBudget(estimatedCost = 0) {
    if (!this.budget) return   // sem limite configurado — libera

    const projectedTotal = this.spent + estimatedCost

    if (projectedTotal >= this.budget * STOP_THRESHOLD) {
      const err = new BudgetExceededError({
        spent:         this.spent,
        budget:        this.budget,
        estimatedNext: estimatedCost,
        projected:     projectedTotal,
      })
      this.onStop?.({ spent: this.spent, budget: this.budget })
      throw err
    }
  }

  // ─── Estima custo da próxima chamada antes de fazê-la ────────────────────

  estimateNextCall(inputTokens, outputTokens = 500) {
    const { calcCost: calc } = require('./pricing.js')  // lazy para evitar circular
    return calcCost(
      { prompt_tokens: inputTokens, completion_tokens: outputTokens },
      this.modelName
    )
  }

  // ─── Relatório final da tarefa ────────────────────────────────────────────

  summary() {
    const totalIn  = this.calls.reduce((s, c) => s + c.promptTokens,     0)
    const totalOut = this.calls.reduce((s, c) => s + c.completionTokens, 0)

    return {
      modelName:    this.modelName,
      budget:       this.budget,
      spent:        this.spent,
      remaining:    this.budget ? Math.max(0, this.budget - this.spent) : null,
      percentUsed:  this.budget ? Math.round((this.spent / this.budget) * 100) : null,
      totalTokens:  { input: totalIn, output: totalOut },
      calls:        this.calls.length,
      breakdown:    this.calls,
    }
  }

  // Versão formatada para mandar ao usuário
  formatSummary() {
    const s = this.summary()
    const lines = [
      `Custo da tarefa: ${formatUSD(s.spent)}`,
      `Tokens: ${s.totalTokens.input.toLocaleString()} entrada + ${s.totalTokens.output.toLocaleString()} saída`,
      `Chamadas ao modelo: ${s.calls}`,
    ]
    if (s.budget) {
      lines.push(`Budget: ${formatUSD(s.spent)} / ${formatUSD(s.budget)} (${s.percentUsed}%)`)
    }
    return lines.join('\n')
  }

  get isFree() {
    return this.spent === 0
  }
}

// ─── Erro especializado para budget excedido ──────────────────────────────────

export class BudgetExceededError extends Error {
  constructor({ spent, budget, estimatedNext, projected }) {
    super(
      `Budget excedido: gasto ${formatUSD(spent)} de ${formatUSD(budget)}. ` +
      `Próxima fase custaria ~${formatUSD(estimatedNext)} (total projetado: ${formatUSD(projected)}).`
    )
    this.name             = 'BudgetExceededError'
    this.spent            = spent
    this.budget           = budget
    this.estimatedNext    = estimatedNext
    this.projected        = projected
  }
}
