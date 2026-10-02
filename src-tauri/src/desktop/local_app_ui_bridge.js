(() => {
  const REQUEST_TYPE = "baijimu:local-app:invoke";
  const RESPONSE_TYPE = "baijimu:local-app:response";
  const READY_TYPE = "baijimu:local-app:ready";
  const HELLO_TYPE = "baijimu:local-app:hello";
  const pending = new Map();
  let sequence = 0;

  const announceReady = () => {
    window.parent.postMessage({ type: READY_TYPE, version: 1 }, "*");
  };

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (message && message.type === HELLO_TYPE && message.version === 1) {
      announceReady();
      return;
    }
    if (!message || message.type !== RESPONSE_TYPE || message.version !== 1) return;
    const request = pending.get(message.requestId);
    if (!request) return;
    pending.delete(message.requestId);
    clearTimeout(request.timeout);
    if (message.ok) request.resolve(message.data);
    else {
      const error = new Error(message.error || "本地应用管理操作失败");
      const details = message.errorDetails;
      if (details && typeof details === "object") {
        if (typeof details.code === "string") error.code = details.code;
        if (details.data !== undefined) error.data = details.data;
        if (details.lifecycle !== undefined) error.lifecycle = details.lifecycle;
      }
      request.reject(error);
    }
  });

  const api = Object.freeze({
    version: 1,
    invoke(operation, payload = null) {
      if (typeof operation !== "string" || !/^[A-Za-z0-9._-]{1,128}$/.test(operation)) {
        return Promise.reject(new Error("management operation 名称无效"));
      }
      const requestId = `${Date.now().toString(36)}-${(++sequence).toString(36)}`;
      return new Promise((resolve, reject) => {
        const timeout = window.setTimeout(() => {
          pending.delete(requestId);
          const error = new Error("本地应用管理请求超时；应用操作可能仍在执行，请先查询状态");
          error.code = "connector_management_timeout";
          reject(error);
        }, 65000);
        pending.set(requestId, { resolve, reject, timeout });
        window.parent.postMessage({
          type: REQUEST_TYPE,
          version: 1,
          requestId,
          operation,
          payload
        }, "*");
      });
    }
  });

  Object.defineProperty(window, "baijimuLocalApp", {
    value: api,
    configurable: false,
    enumerable: true,
    writable: false
  });
  announceReady();
  window.addEventListener("pageshow", announceReady);
})();
