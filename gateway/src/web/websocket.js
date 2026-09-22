(() => {
  const pair = __wsPair,
    action = __wsAction,
    host = __host,
    slots = new WeakMap(),
    internal = Symbol();
  const call = (op, args) => host(op, JSON.stringify(args)).promise;
  const token = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
  class MessageEvent extends Event {
    constructor(type, init = {}) {
      super(type, init);
      Object.defineProperties(this, {
        data: { value: init.data ?? null, enumerable: true },
        source: { value: init.source ?? null, enumerable: true },
        ports: { value: init.ports ?? [], enumerable: true },
        origin: { value: init.origin ?? "", enumerable: true },
        lastEventId: { value: init.lastEventId ?? "", enumerable: true },
      });
    }
  }
  class CloseEvent extends Event {
    constructor(type, init = {}) {
      super(type, init);
      Object.defineProperties(this, {
        code: { value: init.code ?? 0, enumerable: true },
        reason: {
          value: new TextDecoder().decode(
            new TextEncoder().encode(String(init.reason ?? "")),
          ),
          enumerable: true,
        },
        wasClean: { value: init.wasClean ?? false, enumerable: true },
      });
    }
  }
  class ErrorEvent extends Event {
    constructor(type, init = {}) {
      super(type, init);
      Object.defineProperties(this, {
        message: { value: init.message ?? "", enumerable: true },
        error: { value: init.error, enumerable: true },
        filename: { value: init.filename ?? "", enumerable: true },
        lineno: { value: init.lineno ?? 0, enumerable: true },
        colno: { value: init.colno ?? 0, enumerable: true },
      });
    }
  }
  function emit(socket, event) {
    socket.dispatchEvent(event);
    const callback = socket["on" + event.type];
    if (typeof callback === "function") callback.call(socket, event);
  }
  async function pump(socket) {
    const state = slots.get(socket);
    if (state.pumping) return;
    state.pumping = true;
    try {
      for (;;) {
        const message = await call("ws.receive", { id: state.id });
        if (message.kind === "error") {
          emit(
            socket,
            new ErrorEvent("error", {
              message: message.reason,
              error: new Error(message.reason),
            }),
          );
          continue;
        }
        if (message.kind === "close") {
          state.closed = true;
          emit(
            socket,
            new CloseEvent("close", {
              code: message.code,
              reason: message.reason,
              wasClean: message.code !== 1006,
            }),
          );
          action(state.id, "closed", "{}");
          return;
        }
        emit(socket, new MessageEvent("message", { data: message.data }));
      }
    } catch (error) {
      if (!state.closed) {
        state.closed = true;
        emit(
          socket,
          new ErrorEvent("error", { message: error.message, error }),
        );
        emit(socket, new CloseEvent("close", { code: 1006, wasClean: false }));
        try {
          action(state.id, "closed", "{}");
        } catch {}
      }
    }
  }
  class WebSocket extends EventTarget {
    constructor(url, protocols) {
      super();
      const state = {
        id: null,
        url: "",
        accepted: false,
        closed: false,
        pumping: false,
      };
      slots.set(this, state);
      if (url === internal) {
        state.id = protocols;
        return;
      }
      const parsed = new URL(String(url));
      if (
        !["ws:", "wss:"].includes(parsed.protocol) ||
        parsed.hash ||
        parsed.username ||
        parsed.password
      )
        throw new SyntaxError("invalid WebSocket URL");
      state.url = parsed.href;
      protocols =
        protocols === undefined
          ? []
          : typeof protocols === "string"
            ? [protocols]
            : Array.from(protocols);
      if (
        protocols.some((p) => typeof p !== "string" || !token.test(p)) ||
        new Set(protocols).size !== protocols.length
      )
        throw new SyntaxError("invalid or duplicate WebSocket protocols");
      call("ws.connect", {
        url: state.url,
        method: "GET",
        headers: protocols.length
          ? [["sec-websocket-protocol", protocols.join(", ")]]
          : [],
        proxy: "direct",
      })
        .then(async (result) => {
          await call("bodyCancel", { id: result.body });
          if (result.status !== 101 || result.webSocket == null)
            throw new Error(`WebSocket handshake failed: ${result.status}`);
          state.id = result.webSocket;
          if (state.closed) {
            action(
              state.id,
              "close",
              JSON.stringify({ code: 1000, reason: "" }),
            );
            return;
          }
          this.accept();
          emit(this, new Event("open"));
        })
        .catch((error) => {
          state.closed = true;
          emit(
            this,
            new ErrorEvent("error", { message: error.message, error }),
          );
          emit(this, new CloseEvent("close", { code: 1006, wasClean: false }));
        });
    }
    get readyState() {
      const s = slots.get(this);
      if (s.closed) return 3;
      if (s.id === null) return 0;
      return action(s.id, "meta", "{}").state;
    }
    get url() {
      const s = slots.get(this);
      return s.url || (s.id === null ? "" : action(s.id, "meta", "{}").url);
    }
    get protocol() {
      const s = slots.get(this);
      return s.id === null ? "" : action(s.id, "meta", "{}").protocol;
    }
    get extensions() {
      return "";
    }
    get binaryType() {
      return "arraybuffer";
    }
    set binaryType(value) {
      if (value !== "arraybuffer")
        throw new TypeError("only arraybuffer is supported");
    }
    accept() {
      const s = slots.get(this);
      if (s.id === null) throw new TypeError("WebSocket is connecting");
      if (s.accepted) return;
      s.accepted = true;
      action(s.id, "accept", "{}");
      pump(this);
    }
    send(value) {
      const s = slots.get(this);
      if (!s.accepted || this.readyState !== 1)
        throw new TypeError("WebSocket must be accepted and open");
      if (typeof value === "string")
        action(s.id, "text", JSON.stringify({ text: value }));
      else if (value instanceof ArrayBuffer || ArrayBuffer.isView(value))
        action(s.id, "binary", "{}", value);
      else throw new TypeError("send requires text or BufferSource");
    }
    close(code = 1000, reason = "") {
      code = Number(code);
      reason = new TextDecoder().decode(
        new TextEncoder().encode(String(reason)),
      );
      if (
        slots.get(this).url &&
        code !== 1000 &&
        !(code >= 3000 && code <= 4999 && Number.isInteger(code))
      ) {
        const error = new Error("invalid WebSocket close code");
        error.name = "InvalidAccessError";
        throw error;
      }
      if (
        !Number.isInteger(code) ||
        (![
          1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013,
          1014,
        ].includes(code) &&
          !(code >= 3000 && code <= 4999))
      )
        throw new SyntaxError("invalid WebSocket close code");
      if (new TextEncoder().encode(reason).length > 123)
        throw new SyntaxError("close reason exceeds 123 bytes");
      const s = slots.get(this);
      if (s.id === null) {
        s.closed = true;
        return;
      }
      if (this.readyState >= 2) return;
      action(s.id, "close", JSON.stringify({ code, reason }));
    }
  }
  for (const [name, value] of Object.entries({
    CONNECTING: 0,
    OPEN: 1,
    CLOSING: 2,
    CLOSED: 3,
  })) {
    Object.defineProperty(WebSocket, name, { value });
    Object.defineProperty(WebSocket.prototype, name, { value });
  }
  class WebSocketPair {
    *[Symbol.iterator]() {
      yield this[0];
      yield this[1];
    }
    constructor() {
      const [a, b] = pair();
      this[0] = new WebSocket(internal, a);
      this[1] = new WebSocket(internal, b);
    }
  }
  Object.assign(globalThis, {
    WebSocket,
    WebSocketPair,
    MessageEvent,
    CloseEvent,
    ErrorEvent,
  });
  globalThis.__wsFrom = (id) => new WebSocket(internal, id);
  globalThis.__wsId = (socket) => {
    const s = slots.get(socket);
    if (!s || s.id === null || s.accepted)
      throw new TypeError("response requires an unaccepted WebSocket");
    return s.id;
  };
})();
