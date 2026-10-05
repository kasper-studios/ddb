/**
 * DDB Main Bridge — Action Registry Skeleton
 * Реализация обработчиков экшенов хоста
 */

export class ActionRegistry {
  constructor() {
    this.handlers = new Map();
  }

  register(actionName, handler) {
    this.handlers.set(actionName, handler);
  }

  has(actionName) {
    return this.handlers.has(actionName);
  }

  getAvailableActions() {
    return Array.from(this.handlers.keys());
  }

  async execute(actionName, params = {}) {
    if (!this.handlers.has(actionName)) {
      return {
        ok: false,
        action: actionName,
        error: "ACTION_NOT_SUPPORTED",
        message: `Action '${actionName}' is not registered or unavailable on this host`,
        available_actions: this.getAvailableActions()
      };
    }

    try {
      const handler = this.handlers.get(actionName);
      const result = await handler(params);
      return {
        ok: true,
        action: actionName,
        result
      };
    } catch (err) {
      return {
        ok: false,
        action: actionName,
        error: "ACTION_EXECUTION_ERROR",
        message: err.message || String(err)
      };
    }
  }
}
