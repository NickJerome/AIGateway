(() => {
  const host = __host,
    nativeBody = __nativeBody,
    cancelHost = __cancelHost,
    boot = __gatewayBoot;
  delete globalThis.__gatewayBoot;
  const bodies = new WeakMap(),
    streamBodies = new WeakMap(),
    cloneNative = __cloneBody;
  const responseMeta = new WeakMap();
  function readonly(object, names) {
    for (const name of names)
      Object.defineProperty(object, name, { writable: false });
  }
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const forbiddenBodyStatus = new Set([101, 204, 205, 304]);
  const headerList = (headers) =>
    [...headers]
      .filter(([k]) => k !== "set-cookie")
      .concat(headers.getSetCookie().map((v) => ["set-cookie", v]));
  function rpc(op, args, bytes, signal) {
    if (signal?.aborted) return Promise.reject(signal.reason);
    const task = host(op, JSON.stringify(args), bytes);
    const completion = globalThis.__durableInput
      ? __durableInput(task.promise)
      : task.promise;
    if (!signal) return completion;
    return new Promise((resolve, reject) => {
      const abort = () => {
        cancelHost(task.id);
        reject(signal.reason);
      };
      signal.addEventListener("abort", abort, { once: true });
      completion
        .then(resolve, reject)
        .finally(() => signal.removeEventListener("abort", abort));
    });
  }
  function bytes(value) {
    if (typeof value === "string") return encoder.encode(value);
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (ArrayBuffer.isView(value)) {
      // Native host conversion reads internal slots, including DataView.
      return value;
    }
    throw new TypeError("expected string or BufferSource");
  }
  function body(value, headers) {
    if (value == null) return null;
    if (streamBodies.has(value)) {
      const b = streamBodies.get(value);
      usable(b);
      return b;
    }
    if (value instanceof ReadableStream) {
      if (value.locked || value._disturbed)
        throw new TypeError("body stream is locked");
      return { id: null, stream: value, used: false };
    }
    if (value instanceof URLSearchParams) {
      value = value.toString();
      if (!headers.has("content-type"))
        headers.set(
          "content-type",
          "application/x-www-form-urlencoded;charset=UTF-8",
        );
    } else if (typeof value === "string" && !headers.has("content-type"))
      headers.set("content-type", "text/plain;charset=UTF-8");
    return { id: nativeBody(bytes(value)), stream: null, used: false };
  }
  function stream(b) {
    if (!b) return null;
    if (b.publicStream) return b.publicStream;
    let reader;
    b.acquire = () => {
      if (b.id === null) reader ||= b.stream.getReader();
    };
    b.publicStream = new ReadableStream(
      {
        async pull(controller) {
          b.used = true;
          try {
            if (b.id !== null) {
              const data = await rpc(
                "bodyRead",
                { id: b.id },
                undefined,
                b.signal,
              );
              if (data === null) controller.close();
              else controller.enqueue(new Uint8Array(data));
            } else {
              reader ||= b.stream.getReader();
              const next = await reader.read();
              if (next.done) {
                controller.close();
                reader.releaseLock();
              } else controller.enqueue(next.value);
            }
          } catch (e) {
            controller.error(e);
          }
        },
        async cancel(reason) {
          b.used = true;
          if (b.id !== null) await rpc("bodyCancel", { id: b.id });
          else {
            reader ||= b.stream.getReader();
            await reader.cancel(reason);
            reader.releaseLock();
          }
        },
      },
      { highWaterMark: 0 },
    );
    streamBodies.set(b.publicStream, b);
    return b.publicStream;
  }
  function usable(b) {
    if (
      b &&
      (b.used ||
        b.publicStream?.locked ||
        b.stream?.locked ||
        b.publicStream?._disturbed ||
        b.stream?._disturbed)
    )
      throw new TypeError("body is locked or already consumed");
  }
  function cloneBody(b) {
    usable(b);
    if (!b) return [null, null];
    if (b.id !== null) {
      const id = cloneNative(b.id);
      if (id != null)
        return [b, { id, stream: null, used: false, signal: b.signal }];
    }
    const [a, c] = stream(b).tee();
    return [
      { id: null, stream: a, used: false },
      { id: null, stream: c, used: false },
    ];
  }
  function proxyBody(b) {
    usable(b);
    if (!b) return null;
    const original = stream(b);
    b.acquire();
    const reader = original.getReader();
    b.used = true;
    if (b.id !== null)
      return { id: b.id, stream: null, used: false, signal: b.signal };
    return {
      id: null,
      used: false,
      stream: new ReadableStream(
        {
          async pull(c) {
            try {
              const next = await reader.read();
              if (next.done) {
                c.close();
                reader.releaseLock();
              } else c.enqueue(next.value);
            } catch (e) {
              c.error(e);
            }
          },
          async cancel(reason) {
            await reader.cancel(reason);
            reader.releaseLock();
          },
        },
        { highWaterMark: 0 },
      ),
    };
  }
  async function consume(object) {
    const b = bodies.get(object);
    usable(b);
    if (!b) return new ArrayBuffer(0);
    const reader = stream(b).getReader();
    const chunks = [];
    let length = 0;
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        if (!(value instanceof Uint8Array))
          throw new TypeError("body stream requires Uint8Array chunks");
        chunks.push(value);
        length += value.byteLength;
      }
      const result = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) {
        result.set(chunk, offset);
        offset += chunk.byteLength;
      }
      return result.buffer;
    } finally {
      reader.releaseLock();
    }
  }
  class BodyMethods {
    get body() {
      return stream(bodies.get(this));
    }
    get bodyUsed() {
      const b = bodies.get(this);
      return !!(
        b?.used ||
        b?.stream?._disturbed ||
        b?.publicStream?._disturbed
      );
    }
    async arrayBuffer() {
      return consume(this);
    }
    async text() {
      if (
        this.headers.has("content-encoding") &&
        this.headers.get("content-encoding") !== "identity"
      )
        throw new TypeError(
          "encoded body must be explicitly decoded before parsing",
        );
      return new TextDecoder().decode(await consume(this));
    }
    async json() {
      return JSON.parse(await this.text());
    }
  }
  class Request extends BodyMethods {
    constructor(input, init = {}) {
      super();
      const source = input instanceof Request ? input : null;
      this.url = source ? source.url : String(input);
      let method = String(init.method ?? source?.method ?? "GET");
      if (
        !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(method) ||
        /^(CONNECT|TRACE|TRACK)$/i.test(method)
      )
        throw new TypeError("invalid request method");
      if (/^(DELETE|GET|HEAD|OPTIONS|POST|PUT)$/i.test(method))
        method = method.toUpperCase();
      this.method = method;
      this.headers = new Headers(init.headers ?? source?.headers);
      this.signal =
        init.signal ?? source?.signal ?? new AbortController().signal;
      this.redirect = init.redirect ?? source?.redirect ?? "follow";
      if (!["follow", "manual", "error"].includes(this.redirect))
        throw new TypeError("invalid redirect");
      let b;
      const replacement = init.body;
      if (replacement != null) b = body(replacement, this.headers);
      else if (source) {
        b = proxyBody(bodies.get(source));
      } else b = null;
      if ((method === "GET" || method === "HEAD") && b)
        throw new TypeError("GET/HEAD cannot have a body");
      bodies.set(this, b);
      readonly(this, ["url", "method", "headers", "signal", "redirect"]);
    }
    clone() {
      const [a, c] = cloneBody(bodies.get(this));
      bodies.set(this, a);
      const out = new Request(this.url, {
        method: this.method,
        headers: this.headers,
        signal: this.signal,
        redirect: this.redirect,
      });
      bodies.set(out, c);
      return out;
    }
  }
  class Response extends BodyMethods {
    constructor(value = null, init = {}) {
      super();
      this.status = Number(init.status ?? 200);
      if (init.webSocket !== undefined) {
        if (this.status !== 101)
          throw new TypeError("WebSocket requires status 101");
        __wsId(init.webSocket);
        Object.defineProperty(this, "webSocket", {
          value: init.webSocket,
          enumerable: true,
        });
      }
      if (
        !Number.isInteger(this.status) ||
        (this.status < 200 && !(this.status === 101 && init.webSocket)) ||
        this.status > 599
      )
        throw new RangeError("invalid response status");
      this.statusText = String(init.statusText ?? "");
      this.headers = new Headers(init.headers);
      this.url = "";
      this.redirected = false;
      this.type = "default";
      const b = body(value, this.headers);
      if (b && forbiddenBodyStatus.has(this.status))
        throw new TypeError("status forbids a response body");
      bodies.set(this, b);
      if (/[\r\n]/.test(this.statusText))
        throw new TypeError("invalid statusText");
      const meta = {};
      for (const key of [
        "status",
        "statusText",
        "headers",
        "url",
        "redirected",
        "type",
      ]) {
        meta[key] = this[key];
        Object.defineProperty(this, key, { get: () => meta[key] });
      }
      responseMeta.set(this, meta);
    }
    get ok() {
      return this.status >= 200 && this.status < 300;
    }
    clone() {
      const [a, c] = cloneBody(bodies.get(this));
      bodies.set(this, a);
      const out = new Response(null, this);
      bodies.set(out, c);
      Object.assign(responseMeta.get(out), {
        url: this.url,
        redirected: this.redirected,
        type: this.type,
      });
      return out;
    }
    static json(value, init = {}) {
      const text = JSON.stringify(value);
      if (text === undefined) throw new TypeError("not JSON serializable");
      const headers = new Headers(init.headers);
      if (!headers.has("content-type"))
        headers.set("content-type", "application/json");
      return new Response(text, { ...init, headers });
    }
    static redirect(url, status = 302) {
      if (![301, 302, 303, 307, 308].includes(status))
        throw new RangeError("invalid redirect status");
      return new Response(null, { status, headers: { location: String(url) } });
    }
    static error() {
      const r = new Response();
      Object.assign(responseMeta.get(r), { status: 0, type: "error" });
      return r;
    }
  }
  function takeBody(object) {
    const b = bodies.get(object);
    usable(b);
    if (!b) return null;
    if (b.id !== null) {
      stream(b).getReader();
      b.used = true;
      return b.id;
    }
    const reader = stream(b).getReader(),
      id = nativeBody();
    (async () => {
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          if (!(value instanceof Uint8Array))
            throw new TypeError("body stream requires Uint8Array");
          for (let n = 0; n < value.length; n += 65536)
            await rpc("uploadWrite", { id }, value.subarray(n, n + 65536));
        }
        await rpc("uploadClose", { id });
      } catch (e) {
        await rpc("uploadError", { id });
      } finally {
        reader.releaseLock();
      }
    })();
    return id;
  }
  async function doFetch(input, init = {}, allowProxy = false) {
    const request =
      input instanceof Request && Object.keys(init).length === 0
        ? input
        : new Request(input, init);
    if (!allowProxy && init.proxy !== undefined)
      throw new TypeError("proxy is supported by ctx.fetch only");
    await globalThis.__durableOutput?.();
    const data = await rpc(
      "fetch",
      {
        url: request.url,
        method: request.method,
        headers: headerList(request.headers),
        body: takeBody(request),
        redirect: request.redirect,
        proxy: allowProxy ? (init.proxy ?? "direct") : "direct",
      },
      undefined,
      request.signal,
    );
    if (request.signal.aborted) {
      await rpc("bodyCancel", { id: data.body });
      throw request.signal.reason;
    }
    if (data.webSocket != null) void WebSocket;
    const result = new Response(null, {
      status: data.status,
      webSocket: data.webSocket == null ? undefined : __wsFrom(data.webSocket),
      headers: data.headers,
    });
    Object.assign(responseMeta.get(result), {
      url: data.url,
      redirected: data.redirected,
    });
    if (request.method !== "HEAD" && !forbiddenBodyStatus.has(result.status)) {
      bodies.set(result, {
        id: data.body,
        stream: null,
        used: false,
        signal: request.signal,
      });
      request.signal.addEventListener(
        "abort",
        () => {
          rpc("bodyCancel", { id: data.body }).catch(() => {});
        },
        { once: true },
      );
    } else await rpc("bodyCancel", { id: data.body });
    return result;
  }
  const requestAbort = new AbortController();
  Object.defineProperty(globalThis, "__gatewayAbort", {
    value: () => requestAbort.abort(),
  });
  const env = Object.assign(Object.create(null), boot.vars);
  Object.defineProperty(env, "IDENTITY", {
    value: boot.identity == null ? null : Object.freeze(boot.identity),
    enumerable: true,
    writable: false,
    configurable: false,
  });
  function options(value, allowed) {
    if (value == null) return {};
    for (const k of Object.keys(value))
      if (!allowed.includes(k))
        throw new TypeError(`unsupported KV option: ${k}`);
    return value;
  }
  for (const binding of boot.bindings)
    env[binding] = {
      async get(key, opt = {}) {
        opt = typeof opt === "string" ? { type: opt } : options(opt, ["type"]);
        const type = opt.type ?? "text";
        if (!["text", "json", "arrayBuffer"].includes(type))
          throw new TypeError("unsupported KV get type");
        const result = await rpc("kv.get", { binding, key: String(key) });
        if (result === null) return null;
        if (type === "arrayBuffer") return result;
        const text = decoder.decode(result);
        return type === "json" ? JSON.parse(text) : text;
      },
      async put(key, value, opt = {}) {
        options(opt, ["expirationTtl"]);
        const ttl = opt.expirationTtl;
        if (ttl !== undefined && (!Number.isSafeInteger(ttl) || ttl < 60))
          throw new TypeError("expirationTtl must be an integer >= 60");
        await rpc("kv.put", { binding, key: String(key), ttl }, bytes(value));
      },
      async delete(key) {
        await rpc("kv.delete", { binding, key: String(key) });
      },
      async list(opt = {}) {
        options(opt, ["prefix", "limit"]);
        const limit = opt.limit ?? 1000;
        if (!Number.isInteger(limit) || limit < 1 || limit > 1000)
          throw new TypeError("limit must be 1–1000");
        return rpc("kv.list", {
          binding,
          prefix: String(opt.prefix ?? ""),
          limit,
        });
      },
    };
  for (const [name, kind] of Object.entries(boot.unsafe ?? {})) {
    if (kind === "usage")
      env[name] = {
        record(value) {
          if (
            value === null ||
            typeof value !== "object" ||
            Array.isArray(value)
          )
            throw new TypeError("usage record must be an object");
          for (const v of Object.values(value))
            if (
              (!["string", "number", "boolean"].includes(typeof v) &&
                v !== null) ||
              (typeof v === "number" && !Number.isFinite(v))
            )
              throw new TypeError("usage fields must be finite scalars");
          __usageRecord(JSON.stringify(value));
        },
        query(options = {}) {
          return rpc("usage.query", {
            since: options.since ?? 0,
            groupBy: options.groupBy ?? [],
          });
        },
      };
    if (kind === "ir")
      env[name] = __makeIR(
        (request) => {
          if (!(request instanceof Request))
            throw new TypeError("IR.from expects Request");
          const b = bodies.get(request);
          usable(b);
          if (b.id == null)
            throw new TypeError("IR requires native buffered body");
          b.used = true;
          if (b.stream) b.stream.getReader();
          return b.id;
        },
        (id) => stream({ id, stream: null, used: false }),
      );
  }
  for (const level of ["log", "info", "warn", "error", "debug"])
    console[level] = (...args) => {
      let text = args
        .map((v) => {
          try {
            return typeof v === "string" ? v : JSON.stringify(v);
          } catch {
            return String(v);
          }
        })
        .join(" ");
      __recordLog(level, text.slice(0, 65536));
    };
  globalThis.__durableBindings?.(env, boot.durable ?? {});
  globalThis.__gatewayDurableInvoke = (module, ctx) => {
    ctx.fetch = (input, init) => doFetch(input, init, true);
    return __gatewayDurable(module, env, ctx);
  };
  globalThis.__gatewayScheduledInvoke = async (handler, event, ctx) => {
    ctx.fetch = (input, init) => doFetch(input, init, true);
    await handler(JSON.parse(event), env, ctx);
    return { status: 204, headers: [], body: null };
  };
  globalThis.Request = Request;
  globalThis.Response = Response;
  globalThis.fetch = (input, init) => doFetch(input, init);
  Object.defineProperty(globalThis, "__gatewayInvoke", {
    value: async (handler, url, method, ctx) => {
      const request = new Request(url, {
        method,
        headers: boot.headers,
        signal: requestAbort.signal,
      });
      if (boot.body !== null)
        bodies.set(request, { id: boot.body, stream: null, used: false });
      ctx.fetch = (input, init) => doFetch(input, init, true);
      const result = await handler(request, env, ctx);
      if (!(result instanceof Response))
        throw new TypeError("fetch() must return a Response");
      const meta = responseMeta.get(result);
      if (meta.status === 101 && !boot.upgrade)
        throw new TypeError("WebSocket response requires an incoming Upgrade");
      if (meta.status < 200 && meta.status !== 101)
        throw new TypeError("cannot send an error Response");
      return {
        status: meta.status,
        webSocket: result.webSocket ? __wsId(result.webSocket) : null,
        headers: headerList(meta.headers),
        body: takeBody(result),
      };
    },
  });
})();
