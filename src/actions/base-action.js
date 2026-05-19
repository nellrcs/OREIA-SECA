// src/actions/base-action.js
export class BaseAction {
  static actionName = 'base'
  validate(params) {}
  async run(params, context) {}
}
