// src/realtime.js
import { DurableObject as DurableObject2 } from "cloudflare:workers";

// src/auth.js
import { DurableObject } from "cloudflare:workers";
var CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
var credential = (env, id) => env.CREDS.get(env.CREDS.idFromName(id));
var directory = (env) => env.POOL.get(env.POOL.idFromName("directory"));
var account = (s) => ({
  id: s.id,
  state: s.state,
  expiresAt: s.expiresAt,
  accountId: s.accountId,
  cooldown: s.cooldown ?? 0
});
var authBase = (env) => env.AUTH_BASE ?? "https://auth.openai.com";
function jwt(token) {
  if (!token) return {};
  const segment = token.split(".")[1];
  if (!segment) throw new Error("invalid id_token");
  const base64 = segment.replace(/-/g, "+").replace(/_/g, "/");
  const bytes = Uint8Array.from(
    atob(base64 + "=".repeat((4 - base64.length % 4) % 4)),
    (c) => c.charCodeAt(0)
  );
  return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
}
function tokenState(id, old, data) {
  if (!data.access_token) throw new Error("OAuth response lacks access_token");
  const claims = jwt(data.id_token);
  const expires = Number(data.expires_in ?? 3600);
  if (!Number.isFinite(expires) || expires < 0)
    throw new Error("invalid token expiry");
  return {
    ...old,
    id,
    state: "active",
    access: data.access_token,
    refresh: data.refresh_token ?? old?.refresh,
    accountId: claims["https://api.openai.com/auth"]?.chatgpt_account_id ?? claims.chatgpt_account_id ?? old?.accountId,
    expiresAt: Date.now() + expires * 1e3,
    cooldown: 0
  };
}
var Credential = class extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.state = await ctx.storage.get("state");
    });
  }
  async install(state) {
    await this.ctx.storage.put("state", state);
    this.state = state;
    return account(state);
  }
  async status() {
    return this.state ? account(this.state) : null;
  }
  async cooldown(seconds) {
    const state = { ...this.state, cooldown: Date.now() + seconds * 1e3 };
    await this.ctx.storage.put("state", state);
    this.state = state;
  }
  async token() {
    if (!this.state || this.state.state === "dead")
      throw new Error("account unavailable");
    if (this.state.cooldown > Date.now())
      throw new Error("account cooling down");
    if (this.state.expiresAt > Date.now() + 6e4) return this.state;
    if (!this.refreshing)
      this.refreshing = this.refresh().finally(() => {
        this.refreshing = null;
      });
    return this.refreshing;
  }
  async refresh() {
    const response = await this.ctx.fetch(authBase(this.env) + "/oauth/token", {
      method: "POST",
      proxy: this.state.proxy ?? "direct",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "refresh_token",
        client_id: CLIENT_ID,
        refresh_token: this.state.refresh
      })
    });
    const data = await response.json();
    if (!response.ok) {
      if (data.error === "refresh_token_reused" || data.error?.code === "refresh_token_reused")
        await this.install({ ...this.state, state: "dead" });
      throw new Error(
        `refresh failed: ${data.error?.code ?? data.error ?? response.status}`
      );
    }
    const state = tokenState(this.state.id, this.state, data);
    await this.install(state);
    return state;
  }
  async maintain() {
    if (!this.state || this.state.state === "dead" || this.state.cooldown > Date.now())
      return;
    await this.token();
  }
};
var Pool = class extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.polling = /* @__PURE__ */ new Map();
    this.registering = Promise.resolve();
  }
  async register(id) {
    const update = this.registering.then(async () => {
      const ids = await this.ctx.storage.get("accounts") ?? [];
      if (!ids.includes(id)) ids.push(id);
      await this.ctx.storage.put("accounts", ids);
      await this.env.CONFIG.put("accounts", JSON.stringify(ids));
      return ids;
    });
    this.registering = update.catch(() => {
    });
    return update;
  }
  async registerCall(id) {
    const work = this.registering.then(async () => {
      const ids = await this.env.CONFIG.get("calls", { type: "json" }) ?? [];
      if (!ids.includes(id)) ids.push(id);
      await this.env.CONFIG.put("calls", JSON.stringify(ids));
    });
    this.registering = work.catch(() => {
    });
    return work;
  }
  async pruneCalls() {
    const work = this.registering.then(async () => {
      const ids = await this.env.CONFIG.get("calls", { type: "json" }) ?? [];
      const kept = [];
      for (const id of ids)
        if (!await this.env.CALLS.get(this.env.CALLS.idFromName(id)).expire())
          kept.push(id);
      await this.env.CONFIG.put("calls", JSON.stringify(kept));
    });
    this.registering = work.catch(() => {
    });
    return work;
  }
  claimUsage(id) {
    const work = this.registering.then(async () => {
      const key = "usage:" + id;
      if (await this.env.CONFIG.get(key)) return false;
      await this.env.CONFIG.put(key, "1", { expirationTtl: 3600 });
      return true;
    });
    this.registering = work.catch(() => {
    });
    return work;
  }
  async login(proxy = "direct") {
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/usercode",
      {
        method: "POST",
        proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ client_id: CLIENT_ID })
      }
    );
    const data = await response.json();
    if (!response.ok)
      throw new Error(`device authorization failed: ${response.status}`);
    if (!data.device_auth_id || !(data.user_code ?? data.usercode))
      throw new Error("invalid device authorization response");
    const id = crypto.randomUUID(), interval = Math.max(1, Number(data.interval ?? 5)), expires_at = Date.now() + Math.min(900, Number(data.expires_in ?? 900)) * 1e3;
    if (!Number.isFinite(interval) || !Number.isFinite(expires_at))
      throw new Error("invalid device authorization timing");
    const state = {
      id,
      proxy,
      device_auth_id: data.device_auth_id,
      user_code: data.user_code ?? data.usercode,
      interval,
      expires_at,
      next: 0
    };
    await this.ctx.storage.put("login:" + id, state);
    return {
      id,
      user_code: state.user_code,
      verify_url: data.verification_uri ?? authBase(this.env) + "/codex/device",
      expires_at,
      interval
    };
  }
  async poll(id) {
    if (!this.polling.has(id))
      this.polling.set(
        id,
        this.pollOnce(id).finally(() => this.polling.delete(id))
      );
    return this.polling.get(id);
  }
  async pollOnce(id) {
    const key = "login:" + id, state = await this.ctx.storage.get(key);
    if (!state || state.expires_at <= Date.now()) return { state: "expired" };
    if (state.done) return state.done;
    if (state.next > Date.now())
      return {
        state: "slow_down",
        retry_after: Math.ceil((state.next - Date.now()) / 1e3)
      };
    state.next = Date.now() + state.interval * 1e3;
    await this.ctx.storage.put(key, state);
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          device_auth_id: state.device_auth_id,
          user_code: state.user_code
        })
      }
    );
    const data = await response.json();
    if (!response.ok) {
      const code = data.error?.code ?? data.error;
      if (code === "expired_token" || response.status === 410)
        return { state: "expired" };
      if (code === "slow_down" || response.status === 429) {
        state.interval += 5;
        state.next = Date.now() + state.interval * 1e3;
        await this.ctx.storage.put(key, state);
        return { state: "slow_down", retry_after: state.interval };
      }
      if (code === "authorization_pending" || [403, 404].includes(response.status))
        return { state: "pending", retry_after: state.interval };
      throw new Error(`device poll failed: ${code ?? response.status}`);
    }
    if (!data.authorization_code || !data.code_verifier)
      throw new Error("invalid device token response");
    const exchanged = await this.ctx.fetch(
      authBase(this.env) + "/oauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "authorization_code",
          client_id: CLIENT_ID,
          code: data.authorization_code,
          code_verifier: data.code_verifier,
          redirect_uri: authBase(this.env) + "/deviceauth/callback"
        })
      }
    );
    const tokens = await exchanged.json();
    if (!exchanged.ok)
      throw new Error(`token exchange failed: ${exchanged.status}`);
    const claims = jwt(tokens.id_token);
    const accountId = claims["https://api.openai.com/auth"]?.chatgpt_account_id ?? claims.chatgpt_account_id ?? id;
    const installed = await credential(this.env, accountId).install(
      tokenState(accountId, { proxy: state.proxy }, tokens)
    );
    await this.register(accountId);
    state.done = { state: "done", account: installed };
    await this.ctx.storage.put(key, state);
    return state.done;
  }
};

// src/realtime.js
var call = (env, id) => env.CALLS.get(env.CALLS.idFromName(id));
var valid = (id) => /^[A-Za-z0-9_-]{1,128}$/.test(id);
var Call = class extends DurableObject2 {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.state = await ctx.storage.get("call");
    });
  }
  async install(account2, expiresAt) {
    if (this.state && this.state.expiresAt > Date.now())
      throw new Error("call already exists");
    const state = { account: account2, expiresAt, lease: null };
    await this.ctx.storage.put("call", state);
    this.state = state;
  }
  async claim() {
    if (!this.state || this.state.expiresAt <= Date.now())
      return { error: "not_found" };
    if (this.state.lease) return { error: "occupied" };
    const lease = crypto.randomUUID();
    const state = { ...this.state, lease };
    await this.ctx.storage.put("call", state);
    this.state = state;
    return state;
  }
  async done(lease) {
    if (this.state?.lease === lease) {
      const state = { ...this.state, lease: null };
      await this.ctx.storage.put("call", state);
      this.state = state;
    }
  }
  async expire() {
    if (!this.state || this.state.expiresAt <= Date.now()) {
      await this.ctx.storage.delete("call");
      this.state = null;
      return true;
    }
    return false;
  }
};
function headers(request, state, upgrade) {
  const h = new Headers();
  for (const key of [
    "x-session-id",
    "session-id",
    "thread-id",
    "originator",
    "openai-safety-identifier",
    "openai-organization",
    "openai-project",
    "x-oai-attestation"
  ])
    if (request.headers.has(key)) h.set(key, request.headers.get(key));
  if (!h.has("originator")) h.set("originator", "Codex Desktop");
  h.set("authorization", `Bearer ${state.access}`);
  h.set("chatgpt-account-id", state.accountId ?? state.id);
  if (upgrade) {
    h.set("upgrade", "websocket");
    if (request.headers.has("sec-websocket-protocol"))
      h.set(
        "sec-websocket-protocol",
        request.headers.get("sec-websocket-protocol")
      );
  } else {
    h.set(
      "content-type",
      request.headers.get("content-type") ?? "application/sdp"
    );
    if (request.headers.has("openai-alpha"))
      h.set("openai-alpha", request.headers.get("openai-alpha"));
  }
  return h;
}
function upgraded(request) {
  return request.headers.get("upgrade")?.toLowerCase() === "websocket";
}
var required = () => new Response("WebSocket upgrade required", {
  status: 426,
  headers: { upgrade: "websocket" }
});
function registerRealtime(app2) {
  app2.get("/codex/realtime", async (c) => {
    if (c.req.query("call_id")) return sideband(c, c.req.query("call_id"));
    if (!upgraded(c.req.raw)) return required();
    const ids = await c.env.CONFIG.get("accounts", { type: "json" }) ?? [];
    for (const id of ids) {
      let state;
      try {
        state = await credential(c.env, id).token();
      } catch {
        continue;
      }
      const url = new URL(
        c.env.REALTIME_UPSTREAM ?? "https://api.openai.com/v1/realtime"
      );
      url.searchParams.set("model", c.req.query("model") ?? "gpt-realtime");
      const response = await c.executionCtx.fetch(url.href, {
        proxy: state.proxy ?? "direct",
        headers: headers(c.req.raw, state, true),
        signal: c.req.raw.signal
      });
      if ([401, 429].includes(response.status)) {
        await response.body?.cancel();
        await credential(c.env, id).cooldown(30);
        continue;
      }
      if (response.status !== 101) return response;
      return new Response(null, {
        status: 101,
        webSocket: response.webSocket,
        headers: response.headers
      });
    }
    return c.json({ error: "no available account" }, 503);
  });
  app2.post("/codex/realtime/calls", async (c) => {
    const ids = await c.env.CONFIG.get("accounts", { type: "json" }) ?? [];
    const body = await c.req.raw.arrayBuffer();
    for (const id of ids) {
      let state;
      try {
        state = await credential(c.env, id).token();
      } catch {
        continue;
      }
      const response = await c.executionCtx.fetch(
        c.env.REALTIME_BOOTSTRAP ?? "https://chatgpt.com/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas",
        {
          method: "POST",
          headers: headers(c.req.raw, state, false),
          body,
          proxy: state.proxy ?? "direct",
          signal: c.req.raw.signal
        }
      );
      if ([401, 429].includes(response.status)) {
        await response.body?.cancel();
        await credential(c.env, id).cooldown(30);
        continue;
      }
      if (!response.ok) return response;
      const location = response.headers.get("location");
      if (!location) throw new Error("bootstrap response lacks Location");
      const upstream = new URL(
        location,
        c.env.REALTIME_BOOTSTRAP ?? "https://chatgpt.com"
      );
      const idCall = upstream.pathname.split("/").filter(Boolean).at(-1);
      if (!valid(idCall)) throw new Error("invalid upstream call ID");
      await call(c.env, idCall).install(
        id,
        Date.now() + Number(c.env.REALTIME_CALL_TTL_MS ?? 36e5)
      );
      await directory(c.env).registerCall(idCall);
      const output = new Headers(response.headers);
      output.set(
        "location",
        new URL("/codex/realtime/calls/" + idCall, c.req.url).href
      );
      return new Response(response.body, {
        status: response.status,
        headers: output
      });
    }
    return c.json({ error: "no available account" }, 503);
  });
  app2.get("/codex/realtime/calls/:id", (c) => sideband(c, c.req.param("id")));
}
async function sideband(c, id) {
  if (!upgraded(c.req.raw)) return required();
  if (!valid(id)) return c.json({ error: "invalid call ID" }, 400);
  const object = call(c.env, id), claim = await object.claim();
  if (claim.error)
    return c.json(
      { error: claim.error },
      claim.error === "occupied" ? 409 : 404
    );
  try {
    const state = await credential(c.env, claim.account).token();
    const url = new URL(
      c.env.REALTIME_UPSTREAM ?? "https://api.openai.com/v1/realtime"
    );
    url.searchParams.set("intent", "quicksilver");
    url.searchParams.set("call_id", id);
    const response = await c.executionCtx.fetch(url.href, {
      headers: headers(c.req.raw, state, true),
      proxy: state.proxy ?? "direct",
      signal: c.req.raw.signal
    });
    if (response.status !== 101) {
      await object.done(claim.lease);
      return response;
    }
    const pair = new WebSocketPair(), down = pair[1], up = response.webSocket;
    down.accept();
    up.accept();
    let ended = false;
    const done = (source, destination, event) => {
      if (ended) return;
      ended = true;
      clearTimeout(timer);
      try {
        destination.close(
          event.code === 1006 ? 1011 : event.code || 1e3,
          event.reason ?? ""
        );
      } catch {
      }
      c.executionCtx.waitUntil(object.done(claim.lease));
    };
    down.addEventListener("message", (event) => up.send(event.data));
    up.addEventListener("message", (event) => down.send(event.data));
    down.addEventListener("close", (event) => done(down, up, event));
    up.addEventListener("close", (event) => done(up, down, event));
    const timer = setTimeout(
      () => {
        down.close(1e3, "call expired");
        up.close(1e3, "call expired");
        c.executionCtx.waitUntil(object.done(claim.lease));
      },
      Math.max(0, claim.expiresAt - Date.now())
    );
    const output = new Headers();
    if (response.headers.has("sec-websocket-protocol"))
      output.set(
        "sec-websocket-protocol",
        response.headers.get("sec-websocket-protocol")
      );
    return new Response(null, {
      status: 101,
      webSocket: pair[0],
      headers: output
    });
  } catch (error) {
    await object.done(claim.lease);
    throw error;
  }
}

// node_modules/hono/dist/compose.js
var compose = (middleware, onError, onNotFound) => {
  return (context, next) => {
    let index = -1;
    return dispatch(0);
    async function dispatch(i) {
      if (i <= index) {
        throw new Error("next() called multiple times");
      }
      index = i;
      let res;
      let isError = false;
      let handler;
      if (middleware[i]) {
        handler = middleware[i][0][0];
        context.req.routeIndex = i;
      } else {
        handler = i === middleware.length && next || void 0;
      }
      if (handler) {
        try {
          res = await handler(context, () => dispatch(i + 1));
        } catch (err) {
          if (err instanceof Error && onError) {
            context.error = err;
            res = await onError(err, context);
            isError = true;
          } else {
            throw err;
          }
        }
      } else {
        if (context.finalized === false && onNotFound) {
          res = await onNotFound(context);
        }
      }
      if (res && (context.finalized === false || isError)) {
        context.res = res;
      }
      return context;
    }
  };
};

// node_modules/hono/dist/http-exception.js
var HTTPException = class extends Error {
  res;
  status;
  /**
   * Creates an instance of `HTTPException`.
   * @param status - HTTP status code for the exception. Defaults to 500.
   * @param options - Additional options for the exception.
   */
  constructor(status = 500, options) {
    super(options?.message, { cause: options?.cause });
    this.res = options?.res;
    this.status = status;
  }
  /**
   * Returns the response object associated with the exception.
   * If a response object is not provided, a new response is created with the error message and status code.
   * @returns The response object.
   */
  getResponse() {
    if (this.res) {
      const newResponse = new Response(this.res.body, {
        status: this.status,
        headers: this.res.headers
      });
      return newResponse;
    }
    return new Response(this.message, {
      status: this.status
    });
  }
};

// node_modules/hono/dist/request/constants.js
var GET_MATCH_RESULT = /* @__PURE__ */ Symbol();

// node_modules/hono/dist/utils/crypto.js
var sha256 = async (data) => {
  const algorithm = { name: "SHA-256", alias: "sha256" };
  const hash = await createHash(data, algorithm);
  return hash;
};
var createHash = async (data, algorithm) => {
  let sourceBuffer;
  if (ArrayBuffer.isView(data) || data instanceof ArrayBuffer) {
    sourceBuffer = data;
  } else {
    if (typeof data === "object") {
      data = JSON.stringify(data);
    }
    sourceBuffer = new TextEncoder().encode(String(data));
  }
  if (crypto && crypto.subtle) {
    const buffer = await crypto.subtle.digest(
      {
        name: algorithm.name
      },
      sourceBuffer
    );
    const hash = Array.prototype.map.call(new Uint8Array(buffer), (x) => ("00" + x.toString(16)).slice(-2)).join("");
    return hash;
  }
  return null;
};

// node_modules/hono/dist/utils/buffer.js
var constantTimeEqualString = (a, b) => {
  const aLen = a.length;
  const bLen = b.length;
  const maxLen = Math.max(aLen, bLen);
  let out = aLen ^ bLen;
  for (let i = 0; i < maxLen; i++) {
    const aChar = i < aLen ? a.charCodeAt(i) : 0;
    const bChar = i < bLen ? b.charCodeAt(i) : 0;
    out |= aChar ^ bChar;
  }
  return out === 0;
};
var timingSafeEqualString = async (a, b, hashFunction) => {
  if (!hashFunction) {
    hashFunction = sha256;
  }
  const [sa, sb] = await Promise.all([hashFunction(a), hashFunction(b)]);
  if (sa == null || sb == null || typeof sa !== "string" || typeof sb !== "string") {
    return false;
  }
  const hashEqual = constantTimeEqualString(sa, sb);
  const originalEqual = constantTimeEqualString(a, b);
  return hashEqual && originalEqual;
};
var timingSafeEqual = async (a, b, hashFunction) => {
  if (typeof a === "string" && typeof b === "string") {
    return timingSafeEqualString(a, b, hashFunction);
  }
  if (!hashFunction) {
    hashFunction = sha256;
  }
  const [sa, sb] = await Promise.all([hashFunction(a), hashFunction(b)]);
  if (!sa || !sb || typeof sa !== "string" || typeof sb !== "string") {
    return false;
  }
  return timingSafeEqualString(sa, sb);
};
var bufferToFormData = (arrayBuffer, contentType) => {
  const response = new Response(arrayBuffer, {
    headers: {
      // Normalize the media type (case-insensitive) while keeping parameters like the boundary
      "Content-Type": contentType.replace(/^[^;]+/, (mediaType) => mediaType.toLowerCase())
    }
  });
  return response.formData();
};

// node_modules/hono/dist/utils/body.js
var MAX_NESTING_DEPTH = 32;
var MAX_NESTED_OBJECTS = 1e4;
var isRawRequest = (request) => "headers" in request;
var parseBody = async (request, options = /* @__PURE__ */ Object.create(null)) => {
  const { all = false, dot = false } = options;
  const headers2 = isRawRequest(request) ? request.headers : request.raw.headers;
  const contentType = headers2.get("Content-Type");
  const mediaType = contentType?.split(";")[0].trim().toLowerCase();
  if (mediaType === "multipart/form-data" || mediaType === "application/x-www-form-urlencoded") {
    return parseFormData(request, { all, dot });
  }
  return {};
};
async function parseFormData(request, options) {
  if (!isRawRequest(request) && request.bodyCache.formData) {
    return convertFormDataToBodyData(
      await request.bodyCache.formData,
      options
    );
  }
  const headers2 = isRawRequest(request) ? request.headers : request.raw.headers;
  const arrayBuffer = await request.arrayBuffer();
  const formDataPromise = bufferToFormData(arrayBuffer, headers2.get("Content-Type") || "");
  if (!isRawRequest(request)) {
    request.bodyCache.formData = formDataPromise;
  }
  const formData = await formDataPromise;
  if (formData) {
    return convertFormDataToBodyData(formData, options);
  }
  return {};
}
function convertFormDataToBodyData(formData, options) {
  const form = /* @__PURE__ */ Object.create(null);
  const nestingState = { count: 0 };
  formData.forEach((value, key) => {
    const shouldParseAllValues = options.all || key.endsWith("[]");
    if (!shouldParseAllValues) {
      form[key] = value;
    } else {
      handleParsingAllValues(form, key, value);
    }
  });
  if (options.dot) {
    Object.entries(form).forEach(([key, value]) => {
      const shouldParseDotValues = key.includes(".");
      if (shouldParseDotValues) {
        handleParsingNestedValues(form, key, value, nestingState);
        delete form[key];
      }
    });
  }
  return form;
}
var handleParsingAllValues = (form, key, value) => {
  if (form[key] !== void 0) {
    if (Array.isArray(form[key])) {
      ;
      form[key].push(value);
    } else {
      form[key] = [form[key], value];
    }
  } else {
    if (!key.endsWith("[]")) {
      form[key] = value;
    } else {
      form[key] = [value];
    }
  }
};
var handleParsingNestedValues = (form, key, value, state) => {
  if (/(?:^|\.)__proto__\./.test(key)) {
    return;
  }
  let nestedForm = form;
  const keys = key.split(".", MAX_NESTING_DEPTH + 2);
  if (keys.length > MAX_NESTING_DEPTH + 1) {
    throwNestingLimitExceeded();
  }
  keys.forEach((key2, index) => {
    if (index === keys.length - 1) {
      nestedForm[key2] = value;
    } else {
      if (!nestedForm[key2] || typeof nestedForm[key2] !== "object" || Array.isArray(nestedForm[key2]) || nestedForm[key2] instanceof File) {
        if (state.count++ >= MAX_NESTED_OBJECTS) {
          throwNestingLimitExceeded();
        }
        nestedForm[key2] = /* @__PURE__ */ Object.create(null);
      }
      nestedForm = nestedForm[key2];
    }
  });
};
var throwNestingLimitExceeded = () => {
  throw new Error("Nesting limit exceeded");
};

// node_modules/hono/dist/utils/url.js
var splitPath = (path) => {
  const paths = path.split("/");
  if (paths[0] === "") {
    paths.shift();
  }
  return paths;
};
var splitRoutingPath = (routePath) => {
  const { groups, path } = extractGroupsFromPath(routePath);
  const paths = splitPath(path);
  return replaceGroupMarks(paths, groups);
};
var extractGroupsFromPath = (path) => {
  const groups = [];
  path = path.replace(/\{[^}]+\}/g, (match, index) => {
    const mark = `@${index}`;
    groups.push([mark, match]);
    return mark;
  });
  return { groups, path };
};
var replaceGroupMarks = (paths, groups) => {
  for (let i = groups.length - 1; i >= 0; i--) {
    const [mark] = groups[i];
    for (let j = paths.length - 1; j >= 0; j--) {
      if (paths[j].includes(mark)) {
        paths[j] = paths[j].replace(mark, groups[i][1]);
        break;
      }
    }
  }
  return paths;
};
var patternCache = {};
var getPattern = (label, next) => {
  if (label === "*") {
    return "*";
  }
  const match = label.match(/^\:([^\{\}]+)(?:\{(.+)\})?$/);
  if (match) {
    const cacheKey = `${label}#${next}`;
    if (!patternCache[cacheKey]) {
      if (match[2]) {
        patternCache[cacheKey] = next && next[0] !== ":" && next[0] !== "*" ? [cacheKey, match[1], new RegExp(`^${match[2]}(?=/${next})`)] : [label, match[1], new RegExp(`^${match[2]}$`)];
      } else {
        patternCache[cacheKey] = [label, match[1], true];
      }
    }
    return patternCache[cacheKey];
  }
  return null;
};
var tryDecode = (str, decoder) => {
  try {
    return decoder(str);
  } catch {
    return str.replace(/(?:%[0-9A-Fa-f]{2})+/g, (match) => {
      try {
        return decoder(match);
      } catch {
        return match;
      }
    });
  }
};
var tryDecodeURI = (str) => tryDecode(str, decodeURI);
var getPath = (request) => {
  const url = request.url;
  const start = url.indexOf("/", url.indexOf(":") + 4);
  let i = start;
  for (; i < url.length; i++) {
    const charCode = url.charCodeAt(i);
    if (charCode === 37) {
      const queryIndex = url.indexOf("?", i);
      const hashIndex = url.indexOf("#", i);
      const end = queryIndex === -1 ? hashIndex === -1 ? void 0 : hashIndex : hashIndex === -1 ? queryIndex : Math.min(queryIndex, hashIndex);
      const path = url.slice(start, end);
      return tryDecodeURI(path.includes("%25") ? path.replace(/%25/g, "%2525") : path);
    } else if (charCode === 63 || charCode === 35) {
      break;
    }
  }
  return url.slice(start, i);
};
var getPathNoStrict = (request) => {
  const result = getPath(request);
  return result.length > 1 && result.at(-1) === "/" ? result.slice(0, -1) : result;
};
var mergePath = (base, sub, ...rest) => {
  if (rest.length) {
    sub = mergePath(sub, ...rest);
  }
  return `${base?.[0] === "/" ? "" : "/"}${base}${sub === "/" ? "" : `${base?.at(-1) === "/" ? "" : "/"}${sub?.[0] === "/" ? sub.slice(1) : sub}`}`;
};
var checkOptionalParameter = (path) => {
  if (path.charCodeAt(path.length - 1) !== 63 || !path.includes(":")) {
    return null;
  }
  const segments = path.split("/");
  const results = [];
  let basePath = "";
  segments.forEach((segment) => {
    if (segment !== "" && !/\:/.test(segment)) {
      basePath += "/" + segment;
    } else if (/\:/.test(segment)) {
      if (segment.charCodeAt(segment.length - 1) === 63) {
        if (results.length === 0 && basePath === "") {
          results.push("/");
        } else {
          results.push(basePath);
        }
        const optionalSegment = segment.slice(0, -1);
        basePath += "/" + optionalSegment;
        results.push(basePath);
      } else {
        basePath += "/" + segment;
      }
    }
  });
  return results.filter((v, i, a) => a.indexOf(v) === i);
};
var tryDecodeURIComponent = (str) => str.indexOf("%") !== -1 ? tryDecode(str, decodeURIComponent_) : str;
var _decodeURI = (value) => {
  if (value.indexOf("+") !== -1) {
    value = value.replace(/\+/g, " ");
  }
  return tryDecodeURIComponent(value);
};
var _getQueryParam = (url, key, multiple) => {
  const hashIndex = url.indexOf("#", 8);
  if (hashIndex !== -1) {
    url = url.slice(0, hashIndex);
  }
  let encoded;
  if (!multiple && key && key.indexOf("%") === -1 && key.indexOf("+") === -1) {
    let keyIndex2 = url.indexOf("?", 8);
    if (keyIndex2 === -1) {
      return void 0;
    }
    if (!url.startsWith(key, keyIndex2 + 1)) {
      keyIndex2 = url.indexOf(`&${key}`, keyIndex2 + 1);
    }
    while (keyIndex2 !== -1) {
      const trailingKeyCode = url.charCodeAt(keyIndex2 + key.length + 1);
      if (trailingKeyCode === 61) {
        const valueIndex = keyIndex2 + key.length + 2;
        const endIndex = url.indexOf("&", valueIndex);
        return _decodeURI(url.slice(valueIndex, endIndex === -1 ? void 0 : endIndex));
      } else if (trailingKeyCode == 38 || isNaN(trailingKeyCode)) {
        return "";
      }
      keyIndex2 = url.indexOf(`&${key}`, keyIndex2 + 1);
    }
    encoded = /[%+]/.test(url);
    if (!encoded) {
      return void 0;
    }
  }
  const results = /* @__PURE__ */ Object.create(null);
  encoded ??= /[%+]/.test(url);
  let keyIndex = url.indexOf("?", 8);
  while (keyIndex !== -1) {
    const nextKeyIndex = url.indexOf("&", keyIndex + 1);
    let valueIndex = url.indexOf("=", keyIndex);
    if (valueIndex > nextKeyIndex && nextKeyIndex !== -1) {
      valueIndex = -1;
    }
    let name = url.slice(
      keyIndex + 1,
      valueIndex === -1 ? nextKeyIndex === -1 ? void 0 : nextKeyIndex : valueIndex
    );
    if (encoded) {
      name = _decodeURI(name);
    }
    keyIndex = nextKeyIndex;
    if (name === "") {
      continue;
    }
    let value;
    if (valueIndex === -1) {
      value = "";
    } else {
      value = url.slice(valueIndex + 1, nextKeyIndex === -1 ? void 0 : nextKeyIndex);
      if (encoded) {
        value = _decodeURI(value);
      }
    }
    if (multiple) {
      if (!(results[name] && Array.isArray(results[name]))) {
        results[name] = [];
      }
      ;
      results[name].push(value);
    } else {
      results[name] ??= value;
    }
  }
  return key ? results[key] : results;
};
var getQueryParam = _getQueryParam;
var getQueryParams = (url, key) => {
  return _getQueryParam(url, key, true);
};
var decodeURIComponent_ = decodeURIComponent;

// node_modules/hono/dist/request.js
var HonoRequest = class {
  /**
   * `.raw` can get the raw Request object.
   *
   * @see {@link https://hono.dev/docs/api/request#raw}
   *
   * @example
   * ```ts
   * // For Cloudflare Workers
   * app.post('/', async (c) => {
   *   const metadata = c.req.raw.cf?.hostMetadata?
   *   ...
   * })
   * ```
   */
  raw;
  #validatedData;
  // Short name of validatedData
  #matchResult;
  routeIndex = 0;
  /**
   * `.path` can get the pathname of the request.
   *
   * @see {@link https://hono.dev/docs/api/request#path}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const pathname = c.req.path // `/about/me`
   * })
   * ```
   */
  path;
  bodyCache = {};
  constructor(request, path = "/", matchResult = [[]]) {
    this.raw = request;
    this.path = path;
    this.#matchResult = matchResult;
  }
  param(key) {
    return key ? this.#getDecodedParam(key) : this.#getAllDecodedParams();
  }
  #getDecodedParam(key) {
    const paramKey = this.#matchResult[0][this.routeIndex]?.[1][key];
    const param = this.#getParamValue(paramKey);
    return param && tryDecodeURIComponent(param);
  }
  #getAllDecodedParams() {
    const decoded = {};
    const keys = Object.keys(this.#matchResult[0][this.routeIndex]?.[1] ?? {});
    for (const key of keys) {
      const value = this.#getParamValue(this.#matchResult[0][this.routeIndex][1][key]);
      if (value !== void 0) {
        decoded[key] = tryDecodeURIComponent(value);
      }
    }
    return decoded;
  }
  #getParamValue(paramKey) {
    return this.#matchResult[1] ? this.#matchResult[1][paramKey] : paramKey;
  }
  query(key) {
    return getQueryParam(this.url, key);
  }
  queries(key) {
    return getQueryParams(this.url, key);
  }
  header(name) {
    if (name) {
      return this.raw.headers.get(name) ?? void 0;
    }
    const headerData = /* @__PURE__ */ Object.create(null);
    this.raw.headers.forEach((value, key) => {
      headerData[key] = value;
    });
    return headerData;
  }
  async parseBody(options) {
    return parseBody(this, options);
  }
  #cachedBody = (key) => {
    const { bodyCache, raw: raw2 } = this;
    const cachedBody = bodyCache[key];
    if (cachedBody) {
      return cachedBody;
    }
    for (const anyCachedKey in bodyCache) {
      return bodyCache[anyCachedKey].then((body) => {
        if (anyCachedKey === "json") {
          body = JSON.stringify(body);
        }
        const contentType = anyCachedKey === "formData" ? void 0 : raw2.headers.get("content-type");
        return new Response(body, {
          headers: contentType ? { "Content-Type": contentType } : void 0
        })[key]();
      });
    }
    return bodyCache[key] = raw2[key]();
  };
  /**
   * `.json()` can parse Request body of type `application/json`
   *
   * @see {@link https://hono.dev/docs/api/request#json}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.json()
   * })
   * ```
   */
  json() {
    return this.#cachedBody("text").then((text) => JSON.parse(text));
  }
  /**
   * `.text()` can parse Request body of type `text/plain`
   *
   * @see {@link https://hono.dev/docs/api/request#text}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.text()
   * })
   * ```
   */
  text() {
    return this.#cachedBody("text");
  }
  /**
   * `.arrayBuffer()` parse Request body as an `ArrayBuffer`
   *
   * @see {@link https://hono.dev/docs/api/request#arraybuffer}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.arrayBuffer()
   * })
   * ```
   */
  arrayBuffer() {
    return this.#cachedBody("arrayBuffer");
  }
  /**
   * `.bytes()` parses the request body as a `Uint8Array`.
   *
   * @see {@link https://hono.dev/docs/api/request#bytes}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.bytes()
   * })
   * ```
   */
  bytes() {
    return this.#cachedBody("arrayBuffer").then((buffer) => new Uint8Array(buffer));
  }
  /**
   * Parses the request body as a `Blob`.
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.blob();
   * });
   * ```
   * @see https://hono.dev/docs/api/request#blob
   */
  blob() {
    return this.#cachedBody("blob");
  }
  /**
   * Parses the request body as `FormData`.
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.formData();
   * });
   * ```
   * @see https://hono.dev/docs/api/request#formdata
   */
  formData() {
    return this.#cachedBody("formData");
  }
  /**
   * Adds validated data to the request.
   *
   * @param target - The target of the validation.
   * @param data - The validated data to add.
   */
  addValidatedData(target, data) {
    ;
    (this.#validatedData ??= {})[target] = data;
  }
  valid(target) {
    return this.#validatedData?.[target];
  }
  /**
   * `.url()` can get the request url strings.
   *
   * @see {@link https://hono.dev/docs/api/request#url}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const url = c.req.url // `http://localhost:8787/about/me`
   *   ...
   * })
   * ```
   */
  get url() {
    return this.raw.url;
  }
  /**
   * `.method()` can get the method name of the request.
   *
   * @see {@link https://hono.dev/docs/api/request#method}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const method = c.req.method // `GET`
   * })
   * ```
   */
  get method() {
    return this.raw.method;
  }
  get [GET_MATCH_RESULT]() {
    return this.#matchResult;
  }
  /**
   * `.matchedRoutes()` can return a matched route in the handler
   *
   * @deprecated
   *
   * Use matchedRoutes helper defined in "hono/route" instead.
   *
   * @see {@link https://hono.dev/docs/api/request#matchedroutes}
   *
   * @example
   * ```ts
   * app.use('*', async function logger(c, next) {
   *   await next()
   *   c.req.matchedRoutes.forEach(({ handler, method, path }, i) => {
   *     const name = handler.name || (handler.length < 2 ? '[handler]' : '[middleware]')
   *     console.log(
   *       method,
   *       ' ',
   *       path,
   *       ' '.repeat(Math.max(10 - path.length, 0)),
   *       name,
   *       i === c.req.routeIndex ? '<- respond from here' : ''
   *     )
   *   })
   * })
   * ```
   */
  get matchedRoutes() {
    return this.#matchResult[0].map(([[, route]]) => route);
  }
  /**
   * `routePath()` can retrieve the path registered within the handler
   *
   * @deprecated
   *
   * Use routePath helper defined in "hono/route" instead.
   *
   * @see {@link https://hono.dev/docs/api/request#routepath}
   *
   * @example
   * ```ts
   * app.get('/posts/:id', (c) => {
   *   return c.json({ path: c.req.routePath })
   * })
   * ```
   */
  get routePath() {
    return this.#matchResult[0].map(([[, route]]) => route)[this.routeIndex].path;
  }
};

// node_modules/hono/dist/utils/html.js
var HtmlEscapedCallbackPhase = {
  Stringify: 1,
  BeforeStream: 2,
  Stream: 3
};
var raw = (value, callbacks) => {
  const escapedString = new String(value);
  escapedString.isEscaped = true;
  escapedString.callbacks = callbacks;
  return escapedString;
};
var resolveCallback = async (str, phase, preserveCallbacks, context, buffer) => {
  if (typeof str === "object" && !(str instanceof String)) {
    if (!(str instanceof Promise)) {
      str = str.toString();
    }
    if (str instanceof Promise) {
      str = await str;
    }
  }
  const callbacks = str.callbacks;
  if (!callbacks?.length) {
    return Promise.resolve(str);
  }
  if (buffer) {
    buffer[0] += str;
  } else {
    buffer = [str];
  }
  const resStr = Promise.all(callbacks.map((c) => c({ phase, buffer, context }))).then(
    (res) => Promise.all(
      res.filter(Boolean).map((str2) => resolveCallback(str2, phase, false, context, buffer))
    ).then(() => buffer[0])
  );
  if (preserveCallbacks) {
    return raw(await resStr, callbacks);
  } else {
    return resStr;
  }
};

// node_modules/hono/dist/context.js
var TEXT_PLAIN = "text/plain; charset=UTF-8";
var setDefaultContentType = (contentType, headers2) => {
  return {
    "Content-Type": contentType,
    ...headers2
  };
};
var createResponseInstance = (body, init) => new Response(body, init);
var Context = class {
  #rawRequest;
  #req;
  /**
   * `.env` can get bindings (environment variables, secrets, KV namespaces, D1 database, R2 bucket etc.) in Cloudflare Workers.
   *
   * @see {@link https://hono.dev/docs/api/context#env}
   *
   * @example
   * ```ts
   * // Environment object for Cloudflare Workers
   * app.get('*', async c => {
   *   const counter = c.env.COUNTER
   * })
   * ```
   */
  env = {};
  #var;
  finalized = false;
  /**
   * `.error` can get the error object from the middleware if the Handler throws an error.
   *
   * @see {@link https://hono.dev/docs/api/context#error}
   *
   * @example
   * ```ts
   * app.use('*', async (c, next) => {
   *   await next()
   *   if (c.error) {
   *     // do something...
   *   }
   * })
   * ```
   */
  error;
  #status;
  #executionCtx;
  #res;
  #layout;
  #renderer;
  #notFoundHandler;
  #preparedHeaders;
  #matchResult;
  #path;
  /**
   * Creates an instance of the Context class.
   *
   * @param req - The Request object.
   * @param options - Optional configuration options for the context.
   */
  constructor(req, options) {
    this.#rawRequest = req;
    if (options) {
      this.#executionCtx = options.executionCtx;
      this.env = options.env;
      this.#notFoundHandler = options.notFoundHandler;
      this.#path = options.path;
      this.#matchResult = options.matchResult;
    }
  }
  /**
   * `.req` is the instance of {@link HonoRequest}.
   */
  get req() {
    this.#req ??= new HonoRequest(this.#rawRequest, this.#path, this.#matchResult);
    return this.#req;
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#event}
   * The FetchEvent associated with the current request.
   *
   * @throws Will throw an error if the context does not have a FetchEvent.
   */
  get event() {
    if (this.#executionCtx && "respondWith" in this.#executionCtx) {
      return this.#executionCtx;
    } else {
      throw Error("This context has no FetchEvent");
    }
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#executionctx}
   * The ExecutionContext associated with the current request.
   *
   * @throws Will throw an error if the context does not have an ExecutionContext.
   */
  get executionCtx() {
    if (this.#executionCtx) {
      return this.#executionCtx;
    } else {
      throw Error("This context has no ExecutionContext");
    }
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#res}
   * The Response object for the current request.
   */
  get res() {
    return this.#res ||= createResponseInstance(null, {
      headers: this.#preparedHeaders ??= new Headers()
    });
  }
  /**
   * Sets the Response object for the current request.
   *
   * @param _res - The Response object to set.
   */
  set res(_res) {
    if (this.#res && _res) {
      _res = createResponseInstance(_res.body, _res);
      for (const [k, v] of this.#res.headers.entries()) {
        if (k === "content-type") {
          continue;
        }
        if (k === "set-cookie") {
          const cookies = this.#res.headers.getSetCookie();
          _res.headers.delete("set-cookie");
          for (const cookie of cookies) {
            _res.headers.append("set-cookie", cookie);
          }
        } else {
          _res.headers.set(k, v);
        }
      }
    }
    this.#res = _res;
    this.finalized = true;
  }
  /**
   * `.render()` can create a response within a layout.
   *
   * @see {@link https://hono.dev/docs/api/context#render-setrenderer}
   *
   * @example
   * ```ts
   * app.get('/', (c) => {
   *   return c.render('Hello!')
   * })
   * ```
   */
  render = (...args) => {
    this.#renderer ??= (content) => this.html(content);
    return this.#renderer(...args);
  };
  /**
   * Sets the layout for the response.
   *
   * @param layout - The layout to set.
   * @returns The layout function.
   */
  setLayout = (layout) => this.#layout = layout;
  /**
   * Gets the current layout for the response.
   *
   * @returns The current layout function.
   */
  getLayout = () => this.#layout;
  /**
   * `.setRenderer()` can set the layout in the custom middleware.
   *
   * @see {@link https://hono.dev/docs/api/context#render-setrenderer}
   *
   * @example
   * ```tsx
   * app.use('*', async (c, next) => {
   *   c.setRenderer((content) => {
   *     return c.html(
   *       <html>
   *         <body>
   *           <p>{content}</p>
   *         </body>
   *       </html>
   *     )
   *   })
   *   await next()
   * })
   * ```
   */
  setRenderer = (renderer) => {
    this.#renderer = renderer;
  };
  /**
   * `.header()` can set headers.
   *
   * @see {@link https://hono.dev/docs/api/context#header}
   *
   * @example
   * ```ts
   * app.get('/welcome', (c) => {
   *   // Set headers
   *   c.header('X-Message', 'Hello!')
   *   c.header('Content-Type', 'text/plain')
   *
   *   // Append multiple headers using the append option (e.g. Vary)
   *   c.header('Vary', 'Accept-Encoding', { append: true })
   *   c.header('Vary', 'User-Agent', { append: true })
   *
   *   return c.body('Thank you for coming')
   * })
   * ```
   */
  header = (name, value, options) => {
    if (this.finalized) {
      this.#res = createResponseInstance(this.#res.body, this.#res);
    }
    const headers2 = this.#res ? this.#res.headers : this.#preparedHeaders ??= new Headers();
    if (value === void 0) {
      headers2.delete(name);
    } else if (options?.append) {
      headers2.append(name, value);
    } else {
      headers2.set(name, value);
    }
  };
  status = (status) => {
    this.#status = status;
  };
  /**
   * `.set()` can set the value specified by the key.
   *
   * @see {@link https://hono.dev/docs/api/context#set-get}
   *
   * @example
   * ```ts
   * app.use('*', async (c, next) => {
   *   c.set('message', 'Hono is hot!!')
   *   await next()
   * })
   * ```
   */
  set = (key, value) => {
    this.#var ??= /* @__PURE__ */ new Map();
    this.#var.set(key, value);
  };
  /**
   * `.get()` can use the value specified by the key.
   *
   * @see {@link https://hono.dev/docs/api/context#set-get}
   *
   * @example
   * ```ts
   * app.get('/', (c) => {
   *   const message = c.get('message')
   *   return c.text(`The message is "${message}"`)
   * })
   * ```
   */
  get = (key) => {
    return this.#var ? this.#var.get(key) : void 0;
  };
  /**
   * `.var` can access the value of a variable.
   *
   * @see {@link https://hono.dev/docs/api/context#var}
   *
   * @example
   * ```ts
   * const result = c.var.client.oneMethod()
   * ```
   */
  // c.var.propName is a read-only
  get var() {
    if (!this.#var) {
      return {};
    }
    return Object.fromEntries(this.#var);
  }
  #newResponse(data, arg, headers2) {
    let responseHeaders = this.#res ? new Headers(this.#res.headers) : this.#preparedHeaders;
    if (typeof arg === "object" && arg.headers) {
      responseHeaders ??= new Headers();
      for (const [key, value] of new Headers(arg.headers)) {
        if (key === "set-cookie") {
          responseHeaders.append(key, value);
        } else {
          responseHeaders.set(key, value);
        }
      }
    }
    if (headers2) {
      if (!responseHeaders) {
        let count = 0;
        for (const k in headers2) {
          if (++count > 1 || typeof headers2[k] !== "string") {
            responseHeaders = new Headers();
            break;
          }
        }
      }
      if (responseHeaders) {
        for (const k in headers2) {
          const v = headers2[k];
          if (typeof v === "string") {
            responseHeaders.set(k, v);
          } else {
            responseHeaders.delete(k);
            for (const v2 of v) {
              responseHeaders.append(k, v2);
            }
          }
        }
      }
    }
    const status = typeof arg === "number" ? arg : arg?.status ?? this.#status;
    return createResponseInstance(data, {
      status,
      headers: responseHeaders ?? headers2
    });
  }
  newResponse = (...args) => this.#newResponse(...args);
  /**
   * `.body()` can return the HTTP response.
   * You can set headers with `.header()` and set HTTP status code with `.status`.
   * This can also be set in `.text()`, `.json()` and so on.
   *
   * @see {@link https://hono.dev/docs/api/context#body}
   *
   * @example
   * ```ts
   * app.get('/welcome', (c) => {
   *   // Set headers
   *   c.header('X-Message', 'Hello!')
   *   c.header('Content-Type', 'text/plain')
   *   // Set HTTP status code
   *   c.status(201)
   *
   *   // Return the response body
   *   return c.body('Thank you for coming')
   * })
   * ```
   */
  body = (data, arg, headers2) => this.#newResponse(data, arg, headers2);
  /**
   * `.text()` can render text as `Content-Type:text/plain`.
   *
   * @see {@link https://hono.dev/docs/api/context#text}
   *
   * @example
   * ```ts
   * app.get('/say', (c) => {
   *   return c.text('Hello!')
   * })
   * ```
   */
  text = (text, arg, headers2) => {
    return !this.#preparedHeaders && !this.#status && !arg && !headers2 && !this.finalized ? new Response(text) : this.#newResponse(
      text,
      arg,
      setDefaultContentType(TEXT_PLAIN, headers2)
    );
  };
  /**
   * `.json()` can render JSON as `Content-Type:application/json`.
   *
   * @see {@link https://hono.dev/docs/api/context#json}
   *
   * @example
   * ```ts
   * app.get('/api', (c) => {
   *   return c.json({ message: 'Hello!' })
   * })
   * ```
   */
  json = (object, arg, headers2) => {
    return this.#newResponse(
      JSON.stringify(object),
      arg,
      setDefaultContentType("application/json", headers2)
    );
  };
  html = (html, arg, headers2) => {
    const res = (html2) => this.#newResponse(html2, arg, setDefaultContentType("text/html; charset=UTF-8", headers2));
    return typeof html === "object" ? resolveCallback(html, HtmlEscapedCallbackPhase.Stringify, false, {}).then(res) : res(html);
  };
  /**
   * `.redirect()` can Redirect, default status code is 302.
   *
   * @see {@link https://hono.dev/docs/api/context#redirect}
   *
   * @example
   * ```ts
   * app.get('/redirect', (c) => {
   *   return c.redirect('/')
   * })
   * app.get('/redirect-permanently', (c) => {
   *   return c.redirect('/', 301)
   * })
   * ```
   */
  redirect = (location, status) => {
    const locationString = String(location);
    this.header(
      "Location",
      // Multibytes should be encoded
      // eslint-disable-next-line no-control-regex
      !/[^\x00-\xFF]/.test(locationString) ? locationString : encodeURI(locationString)
    );
    return this.newResponse(null, status ?? 302);
  };
  /**
   * `.notFound()` can return the Not Found Response.
   *
   * @see {@link https://hono.dev/docs/api/context#notfound}
   *
   * @example
   * ```ts
   * app.get('/notfound', (c) => {
   *   return c.notFound()
   * })
   * ```
   */
  notFound = () => {
    this.#notFoundHandler ??= () => createResponseInstance();
    return this.#notFoundHandler(this);
  };
};

// node_modules/hono/dist/router.js
var METHOD_NAME_ALL = "ALL";
var METHOD_NAME_ALL_LOWERCASE = "all";
var METHODS = ["get", "post", "put", "delete", "options", "patch", "query"];
var MESSAGE_MATCHER_IS_ALREADY_BUILT = "Can not add a route since the matcher is already built.";
var UnsupportedPathError = class extends Error {
};

// node_modules/hono/dist/utils/constants.js
var COMPOSED_HANDLER = "__COMPOSED_HANDLER";

// node_modules/hono/dist/hono-base.js
var notFoundHandler = (c) => {
  return c.text("404 Not Found", 404);
};
var errorHandler = (err, c) => {
  if ("getResponse" in err) {
    const res = err.getResponse();
    return c.newResponse(res.body, res);
  }
  console.error(err);
  return c.text("Internal Server Error", 500);
};
var Hono = class _Hono {
  get;
  post;
  put;
  delete;
  options;
  patch;
  query;
  all;
  on;
  use;
  /*
    This class is like an abstract class and does not have a router.
    To use it, inherit the class and implement router in the constructor.
  */
  router;
  getPath;
  // Cannot use `#` because it requires visibility at JavaScript runtime.
  _basePath = "/";
  #path = "/";
  routes = [];
  constructor(options = {}) {
    const allMethods = [...METHODS, METHOD_NAME_ALL_LOWERCASE];
    allMethods.forEach((method) => {
      this[method] = (args1, ...args) => {
        const methodName = method.toUpperCase();
        if (typeof args1 === "string") {
          this.#path = args1;
        } else {
          this.#addRoute(methodName, this.#path, args1);
        }
        args.forEach((handler) => {
          this.#addRoute(methodName, this.#path, handler);
        });
        return this;
      };
    });
    this.on = (method, path, ...handlers) => {
      for (const p of [path].flat()) {
        this.#path = p;
        for (const m of [method].flat()) {
          const methodName = m.toUpperCase();
          for (const handler of handlers) {
            this.#addRoute(methodName, this.#path, handler);
          }
        }
      }
      return this;
    };
    this.use = (arg1, ...handlers) => {
      if (typeof arg1 === "string") {
        this.#path = arg1;
      } else {
        this.#path = "*";
        handlers.unshift(arg1);
      }
      handlers.forEach((handler) => {
        this.#addRoute(METHOD_NAME_ALL, this.#path, handler);
      });
      return this;
    };
    const { strict, ...optionsWithoutStrict } = options;
    Object.assign(this, optionsWithoutStrict);
    this.getPath = strict ?? true ? options.getPath ?? getPath : getPathNoStrict;
  }
  #clone() {
    const clone = new _Hono({
      router: this.router,
      getPath: this.getPath
    });
    clone.errorHandler = this.errorHandler;
    clone.#notFoundHandler = this.#notFoundHandler;
    clone.routes = this.routes;
    return clone;
  }
  #notFoundHandler = notFoundHandler;
  // Cannot use `#` because it requires visibility at JavaScript runtime.
  errorHandler = errorHandler;
  /**
   * `.route()` allows grouping other Hono instance in routes.
   *
   * @see {@link https://hono.dev/docs/api/routing#grouping}
   *
   * @param {string} path - base Path
   * @param {Hono} app - other Hono instance
   * @returns {Hono} routed Hono instance
   *
   * @example
   * ```ts
   * const app = new Hono()
   * const app2 = new Hono()
   *
   * app2.get("/user", (c) => c.text("user"))
   * app.route("/api", app2) // GET /api/user
   * ```
   */
  route(path, app2) {
    const subApp = this.basePath(path);
    app2.routes.map((r) => {
      let handler;
      if (app2.errorHandler === errorHandler) {
        handler = r.handler;
      } else {
        handler = async (c, next) => (await compose([], app2.errorHandler)(c, () => r.handler(c, next))).res;
        handler[COMPOSED_HANDLER] = r.handler;
      }
      subApp.#addRoute(r.method, r.path, handler, r.basePath);
    });
    return this;
  }
  /**
   * `.basePath()` allows base paths to be specified.
   *
   * @see {@link https://hono.dev/docs/api/routing#base-path}
   *
   * @param {string} path - base Path
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * const api = new Hono().basePath('/api')
   * ```
   */
  basePath(path) {
    const subApp = this.#clone();
    subApp._basePath = mergePath(this._basePath, path);
    return subApp;
  }
  /**
   * `.onError()` handles an error and returns a customized Response.
   *
   * @see {@link https://hono.dev/docs/api/hono#error-handling}
   *
   * @param {ErrorHandler} handler - request Handler for error
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * app.onError((err, c) => {
   *   console.error(`${err}`)
   *   return c.text('Custom Error Message', 500)
   * })
   * ```
   */
  onError = (handler) => {
    this.errorHandler = handler;
    return this;
  };
  /**
   * `.notFound()` allows you to customize a Not Found Response.
   *
   * @see {@link https://hono.dev/docs/api/hono#not-found}
   *
   * @param {NotFoundHandler} handler - request handler for not-found
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * app.notFound((c) => {
   *   return c.text('Custom 404 Message', 404)
   * })
   * ```
   */
  notFound = (handler) => {
    this.#notFoundHandler = handler;
    return this;
  };
  /**
   * `.mount()` allows you to mount applications built with other frameworks into your Hono application.
   *
   * @see {@link https://hono.dev/docs/api/hono#mount}
   *
   * @param {string} path - base Path
   * @param {Function} applicationHandler - other Request Handler
   * @param {MountOptions} [options] - options of `.mount()`
   * @returns {Hono} mounted Hono instance
   *
   * @example
   * ```ts
   * import { Router as IttyRouter } from 'itty-router'
   * import { Hono } from 'hono'
   * // Create itty-router application
   * const ittyRouter = IttyRouter()
   * // GET /itty-router/hello
   * ittyRouter.get('/hello', () => new Response('Hello from itty-router'))
   *
   * const app = new Hono()
   * app.mount('/itty-router', ittyRouter.handle)
   * ```
   *
   * @example
   * ```ts
   * const app = new Hono()
   * // Send the request to another application without modification.
   * app.mount('/app', anotherApp, {
   *   replaceRequest: (req) => req,
   * })
   * ```
   */
  mount(path, applicationHandler, options) {
    let replaceRequest;
    let optionHandler;
    if (options) {
      if (typeof options === "function") {
        optionHandler = options;
      } else {
        optionHandler = options.optionHandler;
        if (options.replaceRequest === false) {
          replaceRequest = (request) => request;
        } else {
          replaceRequest = options.replaceRequest;
        }
      }
    }
    const getOptions = optionHandler ? (c) => {
      const options2 = optionHandler(c);
      return Array.isArray(options2) ? options2 : [options2];
    } : (c) => {
      let executionContext = void 0;
      try {
        executionContext = c.executionCtx;
      } catch {
      }
      return [c.env, executionContext];
    };
    replaceRequest ||= (() => {
      const mergedPath = mergePath(this._basePath, path);
      const pathPrefixLength = mergedPath === "/" ? 0 : mergedPath.length;
      return (request) => {
        const url = new URL(request.url);
        url.pathname = this.getPath(request).slice(pathPrefixLength) || "/";
        return new Request(url, request);
      };
    })();
    const handler = async (c, next) => {
      const res = await applicationHandler(replaceRequest(c.req.raw), ...getOptions(c));
      if (res) {
        return res;
      }
      await next();
    };
    this.#addRoute(METHOD_NAME_ALL, mergePath(path, "*"), handler);
    return this;
  }
  #addRoute(method, path, handler, baseRoutePath) {
    path = mergePath(this._basePath, path);
    const r = {
      basePath: baseRoutePath !== void 0 ? mergePath(this._basePath, baseRoutePath) : this._basePath,
      path,
      method,
      handler
    };
    this.router.add(method, path, [handler, r]);
    this.routes.push(r);
  }
  #handleError(err, c) {
    if (err instanceof Error) {
      return this.errorHandler(err, c);
    }
    throw err;
  }
  #dispatch(request, executionCtx, env, method) {
    if (method === "HEAD") {
      return (async () => new Response(null, await this.#dispatch(request, executionCtx, env, "GET")))();
    }
    const path = this.getPath(request, { env });
    const matchResult = this.router.match(method, path);
    const c = new Context(request, {
      path,
      matchResult,
      env,
      executionCtx,
      notFoundHandler: this.#notFoundHandler
    });
    if (matchResult[0].length === 1) {
      let res;
      try {
        res = matchResult[0][0][0][0](c, async () => {
          c.res = await this.#notFoundHandler(c);
        });
      } catch (err) {
        return this.#handleError(err, c);
      }
      return res instanceof Promise ? res.then(
        (resolved) => resolved || (c.finalized ? c.res : this.#notFoundHandler(c))
      ).catch((err) => this.#handleError(err, c)) : res ?? this.#notFoundHandler(c);
    }
    const composed = compose(matchResult[0], this.errorHandler, this.#notFoundHandler);
    return (async () => {
      try {
        const context = await composed(c);
        if (!context.finalized) {
          throw new Error(
            "Context is not finalized. Did you forget to return a Response object or `await next()`?"
          );
        }
        return context.res;
      } catch (err) {
        return this.#handleError(err, c);
      }
    })();
  }
  /**
   * `.fetch()` will be entry point of your app.
   *
   * @see {@link https://hono.dev/docs/api/hono#fetch}
   *
   * @param {Request} request - request Object of request
   * @param {Env} env - env Object
   * @param {ExecutionContext} executionCtx - context of execution
   * @returns {Response | Promise<Response>} response of request
   *
   */
  fetch = (request, ...rest) => {
    return this.#dispatch(request, rest[1], rest[0], request.method);
  };
  /**
   * `.request()` is a useful method for testing.
   * You can pass a URL or pathname to send a GET request.
   * app will return a Response object.
   * ```ts
   * test('GET /hello is ok', async () => {
   *   const res = await app.request('/hello')
   *   expect(res.status).toBe(200)
   * })
   * ```
   * @see https://hono.dev/docs/api/hono#request
   */
  request = (input, requestInit, Env, executionCtx) => {
    if (input instanceof Request) {
      return this.fetch(requestInit ? new Request(input, requestInit) : input, Env, executionCtx);
    }
    input = input.toString();
    return this.fetch(
      new Request(
        /^https?:\/\//.test(input) ? input : `http://localhost${mergePath("/", input)}`,
        requestInit
      ),
      Env,
      executionCtx
    );
  };
  /**
   * `.fire()` automatically adds a global fetch event listener.
   * This can be useful for environments that adhere to the Service Worker API, such as non-ES module Cloudflare Workers.
   * @deprecated
   * Use `fire` from `hono/service-worker` instead.
   * ```ts
   * import { Hono } from 'hono'
   * import { fire } from 'hono/service-worker'
   *
   * const app = new Hono()
   * // ...
   * fire(app)
   * ```
   * @see https://hono.dev/docs/api/hono#fire
   * @see https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API
   * @see https://developers.cloudflare.com/workers/reference/migrate-to-module-workers/
   */
  fire = () => {
    addEventListener("fetch", (event) => {
      event.respondWith(this.#dispatch(event.request, event, void 0, event.request.method));
    });
  };
};

// node_modules/hono/dist/router/linear-router/router.js
var emptyParams = /* @__PURE__ */ Object.create(null);
var splitPathRe = /\/(:\w+(?:{(?:(?:{[\d,]+})|[^}])+})?)|\/[^\/\?]+|(\?)/g;
var splitByStarRe = /\*/;
var LinearRouter = class {
  name = "LinearRouter";
  #routes = [];
  add(method, path, handler) {
    for (let i = 0, paths = checkOptionalParameter(path) || [path], len = paths.length; i < len; i++) {
      this.#routes.push([method, paths[i], handler]);
    }
  }
  match(method, path) {
    const handlers = [];
    ROUTES_LOOP: for (let i = 0, len = this.#routes.length; i < len; i++) {
      const [routeMethod, routePath, handler] = this.#routes[i];
      if (routeMethod === method || routeMethod === METHOD_NAME_ALL) {
        if (routePath === "*" || routePath === "/*") {
          handlers.push([handler, emptyParams]);
          continue;
        }
        const hasStar = routePath.indexOf("*") !== -1;
        const hasLabel = routePath.indexOf(":") !== -1;
        if (!hasStar && !hasLabel) {
          if (routePath === path || routePath + "/" === path) {
            handlers.push([handler, emptyParams]);
          }
        } else if (hasStar && !hasLabel) {
          const endsWithStar = routePath.charCodeAt(routePath.length - 1) === 42;
          const endsWithSlashStar = routePath.endsWith("/*");
          const parts = (endsWithStar ? routePath.slice(0, endsWithSlashStar ? -2 : -1) : routePath).split(splitByStarRe);
          const lastIndex = parts.length - 1;
          for (let j = 0, pos = 0, len2 = parts.length; j < len2; j++) {
            const part = parts[j];
            const index = path.indexOf(part, pos);
            if (index !== pos) {
              continue ROUTES_LOOP;
            }
            pos += part.length;
            if (j === lastIndex) {
              if (endsWithSlashStar) {
                if (pos !== path.length && path.charCodeAt(pos) !== 47) {
                  continue ROUTES_LOOP;
                }
              } else if (!endsWithStar && pos !== path.length && !(pos === path.length - 1 && path.charCodeAt(pos) === 47)) {
                continue ROUTES_LOOP;
              }
            } else {
              const index2 = path.indexOf("/", pos);
              if (index2 === -1) {
                continue ROUTES_LOOP;
              }
              pos = index2;
            }
          }
          handlers.push([handler, emptyParams]);
        } else if (hasLabel && !hasStar) {
          const params = /* @__PURE__ */ Object.create(null);
          const parts = routePath.match(splitPathRe);
          const lastIndex = parts.length - 1;
          for (let j = 0, pos = 0, len2 = parts.length; j < len2; j++) {
            if (pos === -1 || pos >= path.length) {
              continue ROUTES_LOOP;
            }
            const part = parts[j];
            if (part.charCodeAt(1) === 58) {
              if (path.charCodeAt(pos) !== 47) {
                continue ROUTES_LOOP;
              }
              let name = part.slice(2);
              let value;
              if (name.charCodeAt(name.length - 1) === 125) {
                const openBracePos = name.indexOf("{");
                const next = parts[j + 1];
                const lookahead = next && next[1] !== ":" && next[1] !== "*" ? `(?=${next})` : "";
                const pattern = name.slice(openBracePos + 1, -1) + lookahead;
                const restPath = path.slice(pos + 1);
                const match = new RegExp(pattern, "d").exec(restPath);
                if (!match || match.indices[0][0] !== 0 || match.indices[0][1] === 0) {
                  continue ROUTES_LOOP;
                }
                name = name.slice(0, openBracePos);
                value = restPath.slice(...match.indices[0]);
                pos += match.indices[0][1] + 1;
              } else {
                let endValuePos = path.indexOf("/", pos + 1);
                if (endValuePos === -1) {
                  if (pos + 1 === path.length) {
                    continue ROUTES_LOOP;
                  }
                  endValuePos = path.length;
                }
                value = path.slice(pos + 1, endValuePos);
                pos = endValuePos;
              }
              params[name] ||= value;
            } else {
              const index = path.indexOf(part, pos);
              if (index !== pos) {
                continue ROUTES_LOOP;
              }
              pos += part.length;
            }
            if (j === lastIndex) {
              if (pos !== path.length && !(pos === path.length - 1 && path.charCodeAt(pos) === 47)) {
                continue ROUTES_LOOP;
              }
            }
          }
          handlers.push([handler, params]);
        } else if (hasLabel && hasStar) {
          throw new UnsupportedPathError();
        }
      }
    }
    return [handlers];
  }
};

// node_modules/hono/dist/router/smart-router/router.js
var SmartRouter = class {
  name = "SmartRouter";
  #routers = [];
  #routes = [];
  constructor(init) {
    this.#routers = init.routers;
  }
  add(method, path, handler) {
    if (!this.#routes) {
      throw new Error(MESSAGE_MATCHER_IS_ALREADY_BUILT);
    }
    this.#routes.push([method, path, handler]);
  }
  match(method, path) {
    if (!this.#routes) {
      throw new Error("Fatal error");
    }
    const routers = this.#routers;
    const routes = this.#routes;
    const len = routers.length;
    let i = 0;
    let res;
    for (; i < len; i++) {
      const router = routers[i];
      try {
        for (let i2 = 0, len2 = routes.length; i2 < len2; i2++) {
          router.add(...routes[i2]);
        }
        res = router.match(method, path);
      } catch (e) {
        if (e instanceof UnsupportedPathError) {
          continue;
        }
        throw e;
      }
      this.match = router.match.bind(router);
      this.#routers = [router];
      this.#routes = void 0;
      break;
    }
    if (i === len) {
      throw new Error("Fatal error");
    }
    this.name = `SmartRouter + ${this.activeRouter.name}`;
    return res;
  }
  get activeRouter() {
    if (this.#routes || this.#routers.length !== 1) {
      throw new Error("No active router has been determined yet.");
    }
    return this.#routers[0];
  }
};

// node_modules/hono/dist/router/utils.js
var createNullObject = () => /* @__PURE__ */ Object.create(null);

// node_modules/hono/dist/router/trie-router/node.js
var emptyParams2 = createNullObject();
var order = 0;
var Node = class _Node {
  #methods = [];
  #children = createNullObject();
  #patterns = [];
  #pattern;
  #params = emptyParams2;
  insert(method, path, handler) {
    let curNode = this;
    const parts = splitRoutingPath(path);
    const possibleKeys = /* @__PURE__ */ new Set();
    let i = 0;
    for (const p of parts) {
      const nextP = parts[++i];
      const pattern = getPattern(p, nextP) || (nextP === void 0 && p && p.indexOf("*") === p.length - 1 ? p : null);
      const isParam = Array.isArray(pattern);
      const key = isParam ? pattern[0] : pattern || p;
      const child = curNode.#children[key] ||= new _Node();
      if (pattern && !child.#pattern) {
        child.#pattern = pattern;
        curNode.#patterns.push(child);
      }
      curNode = child;
      if (isParam) {
        possibleKeys.add(pattern[1]);
      }
    }
    curNode.#methods.push({
      [method]: {
        handler,
        possibleKeys: [...possibleKeys],
        score: ++order
      }
    });
  }
  #pushHandlerSets(handlerSets, node, method, nodeParams, params) {
    for (let i = 0, len = node.#methods.length; i < len; i++) {
      const m = node.#methods[i];
      const handlerSet = m[method] || m[METHOD_NAME_ALL];
      if (handlerSet) {
        handlerSet.params = createNullObject();
        handlerSets.push(handlerSet);
        for (let i2 = 0, len2 = handlerSet.possibleKeys.length; i2 < len2; i2++) {
          const key = handlerSet.possibleKeys[i2];
          handlerSet.params[key] = params?.[key] && !i2 ? params[key] : nodeParams[key] ?? params?.[key];
        }
      }
    }
  }
  search(method, path) {
    const handlerSets = [];
    this.#params = emptyParams2;
    const curNode = this;
    let curNodes = [curNode];
    const parts = splitPath(path);
    const curNodesQueue = [];
    const len = parts.length;
    let partOffsets = null;
    for (let i = 0; i < len; i++) {
      const part = parts[i];
      const isLast = i === len - 1;
      const tempNodes = [];
      for (let j = 0, len2 = curNodes.length; j < len2; j++) {
        const node = curNodes[j];
        const nextNode = node.#children[part];
        if (nextNode) {
          nextNode.#params = node.#params;
          if (isLast) {
            if (nextNode.#children["*"]) {
              this.#pushHandlerSets(handlerSets, nextNode.#children["*"], method, node.#params);
            }
            this.#pushHandlerSets(handlerSets, nextNode, method, node.#params);
          } else {
            tempNodes.push(nextNode);
          }
        }
        for (const child of node.#patterns) {
          const pattern = child.#pattern;
          const params = node.#params === emptyParams2 ? {} : { ...node.#params };
          if (typeof pattern === "string") {
            if (pattern === "*" || part.startsWith(pattern.slice(0, -1))) {
              this.#pushHandlerSets(handlerSets, child, method, node.#params);
              if (pattern === "*") {
                child.#params = params;
                tempNodes.push(child);
              }
            }
            continue;
          }
          const [, name, matcher] = pattern;
          if (!part && matcher === true) {
            continue;
          }
          if (matcher !== true) {
            if (!partOffsets) {
              partOffsets = [];
              let offset = path[0] === "/" ? 1 : 0;
              for (let p = 0; p < len; p++) {
                partOffsets[p] = offset;
                offset += parts[p].length + 1;
              }
            }
            const restPathString = path.slice(partOffsets[i]);
            const m = matcher.exec(restPathString);
            if (m) {
              params[name] = m[0];
              this.#pushHandlerSets(handlerSets, child, method, node.#params, params);
              if (m[0].length === restPathString.length && child.#children["*"]) {
                this.#pushHandlerSets(
                  handlerSets,
                  child.#children["*"],
                  method,
                  node.#params,
                  params
                );
              }
              for (const _ in child.#children) {
                child.#params = params;
                const componentCount = m[0].match(/\//g)?.length ?? 0;
                const targetCurNodes = curNodesQueue[componentCount] ||= [];
                targetCurNodes.push(child);
                break;
              }
              continue;
            }
          }
          if (matcher === true || matcher.test(part)) {
            params[name] = part;
            if (isLast) {
              this.#pushHandlerSets(handlerSets, child, method, params, node.#params);
              if (child.#children["*"]) {
                this.#pushHandlerSets(
                  handlerSets,
                  child.#children["*"],
                  method,
                  params,
                  node.#params
                );
              }
            } else {
              child.#params = params;
              tempNodes.push(child);
            }
          }
        }
      }
      const shifted = curNodesQueue.shift();
      curNodes = shifted ? tempNodes.concat(shifted) : tempNodes;
    }
    if (handlerSets[1]) {
      handlerSets.sort((a, b) => {
        return a.score - b.score;
      });
    }
    return [handlerSets.map(({ handler, params }) => [handler, params])];
  }
};

// node_modules/hono/dist/router/trie-router/router.js
var TrieRouter = class {
  name = "TrieRouter";
  #node = new Node();
  add(method, path, handler) {
    for (const result of checkOptionalParameter(path) || [path]) {
      this.#node.insert(method, result, handler);
    }
  }
  match(method, path) {
    return this.#node.search(method, path);
  }
};

// node_modules/hono/dist/preset/quick.js
var Hono2 = class extends Hono {
  constructor(options = {}) {
    super(options);
    this.router = new SmartRouter({
      routers: [new LinearRouter(), new TrieRouter()]
    });
  }
};

// node_modules/hono/dist/middleware/bearer-auth/index.js
var TOKEN_STRINGS = "[A-Za-z0-9._~+/-]+=*";
var PREFIX = "Bearer";
var HEADER = "Authorization";
var bearerAuth = (options) => {
  if (!("token" in options || "verifyToken" in options)) {
    throw new Error('bearer auth middleware requires options for "token" or "verifyToken"');
  }
  if (!options.realm) {
    options.realm = "";
  }
  if (options.prefix === void 0) {
    options.prefix = PREFIX;
  }
  const realm = options.realm?.replace(/"/g, '\\"');
  const prefix = options.prefix;
  const tokenRegexp = new RegExp(`^${TOKEN_STRINGS}$`);
  const wwwAuthenticatePrefix = prefix === "" ? "" : `${prefix} `;
  const throwHTTPException = async (c, status, wwwAuthenticateHeader, messageOption) => {
    const wwwAuthenticateHeaderValue = typeof wwwAuthenticateHeader === "function" ? await wwwAuthenticateHeader(c) : wwwAuthenticateHeader;
    const headers2 = {
      "WWW-Authenticate": typeof wwwAuthenticateHeaderValue === "string" ? wwwAuthenticateHeaderValue : `${wwwAuthenticatePrefix}${Object.entries(wwwAuthenticateHeaderValue).map(([key, value]) => `${key}="${value}"`).join(",")}`
    };
    const responseMessage = typeof messageOption === "function" ? await messageOption(c) : messageOption;
    const res = typeof responseMessage === "string" ? new Response(responseMessage, { status, headers: headers2 }) : new Response(JSON.stringify(responseMessage), {
      status,
      headers: {
        ...headers2,
        "content-type": "application/json"
      }
    });
    throw new HTTPException(status, { res });
  };
  return async function bearerAuth2(c, next) {
    const headerToken = c.req.header(options.headerName || HEADER);
    if (!headerToken) {
      await throwHTTPException(
        c,
        401,
        options.noAuthenticationHeader?.wwwAuthenticateHeader || `${wwwAuthenticatePrefix}realm="${realm}"`,
        options.noAuthenticationHeader?.message || options.noAuthenticationHeaderMessage || "Unauthorized"
      );
    } else {
      let tokenValue;
      if (prefix === "") {
        tokenValue = headerToken;
      } else {
        const headerLower = headerToken.toLowerCase();
        const prefixLower = prefix.toLowerCase();
        if (headerLower.startsWith(prefixLower) && headerToken[prefix.length] === " ") {
          tokenValue = headerToken.slice(prefix.length).trimStart();
        }
      }
      if (!tokenValue || !tokenRegexp.test(tokenValue)) {
        await throwHTTPException(
          c,
          400,
          options.invalidAuthenticationHeader?.wwwAuthenticateHeader || `${wwwAuthenticatePrefix}error="invalid_request"`,
          options.invalidAuthenticationHeader?.message || options.invalidAuthenticationHeaderMessage || "Bad Request"
        );
      } else {
        let equal = false;
        if ("verifyToken" in options) {
          equal = await options.verifyToken(tokenValue, c);
        } else if (typeof options.token === "string") {
          equal = await timingSafeEqual(options.token, tokenValue, options.hashFunction);
        } else if (Array.isArray(options.token) && options.token.length > 0) {
          for (const token of options.token) {
            if (await timingSafeEqual(token, tokenValue, options.hashFunction)) {
              equal = true;
              break;
            }
          }
        }
        if (!equal) {
          await throwHTTPException(
            c,
            401,
            options.invalidToken?.wwwAuthenticateHeader || `${wwwAuthenticatePrefix}error="invalid_token"`,
            options.invalidToken?.message || options.invalidTokenMessage || "Unauthorized"
          );
        }
      }
    }
    await next();
  };
};

// src/sse.js
async function remember(env, id, acct) {
  if (id) await env.CONFIG.put("response:" + id, acct, { expirationTtl: 3600 });
}
async function recordUsage(env, value, acct) {
  if (!value?.id || !value.usage) return;
  if (!await directory(env).claimUsage(value.id)) return;
  const u = value.usage;
  const record = {
    credential: acct,
    model: value.model ?? "",
    input: u.input_tokens,
    output: u.output_tokens,
    cached: u.input_tokens_details?.cached_tokens,
    reasoning: u.output_tokens_details?.reasoning_tokens
  };
  env.USAGE.record(
    Object.fromEntries(
      Object.entries(record).filter(([, v]) => v !== void 0)
    )
  );
}
function inspectSSE(body, env, acct) {
  let pending = "";
  async function inspect(frame) {
    const text = frame.split(/\r?\n/).filter((l) => l.startsWith("data:")).map((l) => l.slice(5).replace(/^ /, "")).join("\n");
    if (text && text !== "[DONE]") {
      let value;
      try {
        value = JSON.parse(text);
      } catch {
      }
      if (value?.response?.id) await remember(env, value.response.id, acct);
      if (value?.type === "response.completed")
        await recordUsage(env, value.response, acct);
    }
  }
  return body.pipeThrough(new TextDecoderStream()).pipeThrough(
    new TransformStream({
      async transform(chunk, c) {
        pending += chunk;
        for (; ; ) {
          const end = /\r?\n\r?\n/.exec(pending);
          if (!end) break;
          const frame = pending.slice(0, end.index + end[0].length);
          pending = pending.slice(frame.length);
          if (frame.length > 1 << 20) throw new Error("SSE frame too large");
          await inspect(frame);
          c.enqueue(frame);
        }
        if (pending.length > 1 << 20) throw new Error("SSE frame too large");
      },
      async flush(c) {
        if (pending) {
          await inspect(pending);
          c.enqueue(pending);
        }
      }
    })
  ).pipeThrough(new TextEncoderStream());
}

// src/index.js
var app = new Hono2();
registerRealtime(app);
app.use("/codex/admin/*", async (c, next) => {
  if (!c.env.ADMIN_TOKEN)
    return c.json({ error: "administrator token is not configured" }, 401);
  return bearerAuth({ token: c.env.ADMIN_TOKEN })(c, next);
});
app.get("/codex/admin/accounts", async (c) => {
  const ids = await c.env.CONFIG.get("accounts", { type: "json" }) ?? [];
  return c.json(
    await Promise.all(ids.map((id) => credential(c.env, id).status()))
  );
});
app.get(
  "/codex/admin/usage",
  async (c) => c.json(
    await c.env.USAGE.query({
      since: Number(c.req.query("since") ?? 0),
      groupBy: ["credential", "model"]
    })
  )
);
app.post(
  "/codex/admin/accounts/login",
  async (c) => c.json(await directory(c.env).login(c.req.query("proxy") ?? "direct"))
);
app.post("/codex/admin/accounts/login/:id/poll", async (c) => {
  const result = await directory(c.env).poll(c.req.param("id"));
  return c.json(result, result.state === "expired" ? 410 : 200);
});
app.post("/codex/responses", async (c) => {
  const request = c.req.raw, env = c.env, ir = env.IR.from(request, "openai.responses");
  const ids = await env.CONFIG.get("accounts", { type: "json" }) ?? [];
  const hard = ir.sessionKeys.previousResponseId;
  const soft = ir.sessionKeys.promptCacheKey ?? ir.sessionKeys.sessionId ?? ir.sessionKeys.threadId;
  const mapped = hard ? await env.CONFIG.get("response:" + hard) : soft ? await env.CONFIG.get("soft:" + soft) : null;
  if (hard && !mapped)
    return c.json(
      { error: "previous_response_id has no account mapping" },
      409
    );
  const candidates = hard ? [mapped] : [...mapped ? [mapped] : [], ...ids.filter((id) => id !== mapped)];
  if (env.MODEL_MAP?.[ir.model]) ir.model = env.MODEL_MAP[ir.model];
  for (const id of candidates) {
    if (request.signal.aborted) throw request.signal.reason;
    let state;
    try {
      state = await credential(env, id).token();
    } catch {
      if (hard) return c.json({ error: "bound account unavailable" }, 503);
      continue;
    }
    const response = await c.executionCtx.fetch(env.UPSTREAM, {
      method: "POST",
      proxy: state.proxy ?? "direct",
      headers: {
        authorization: `Bearer ${state.access}`,
        "chatgpt-account-id": state.accountId ?? id,
        "content-type": "application/json",
        accept: "text/event-stream"
      },
      body: env.IR.to(ir),
      signal: request.signal
    });
    if ([401, 429].includes(response.status)) {
      await response.body?.cancel();
      await credential(env, id).cooldown(response.status === 429 ? 60 : 30);
      if (hard) return c.json({ error: "bound account rejected request" }, 503);
      continue;
    }
    if (!response.ok) return response;
    if (soft) {
      await env.CONFIG.put("soft:" + soft, id, { expirationTtl: 3600 });
      if (mapped && mapped !== id)
        env.USAGE.record({
          kind: "soft_fallback",
          credential: id,
          model: ir.model ?? "",
          fallback: 1
        });
    }
    if (response.headers.get("content-type")?.includes("text/event-stream"))
      return new Response(inspectSSE(response.body, env, id), {
        status: response.status,
        headers: response.headers
      });
    const value = await response.json();
    await remember(env, value.id, id);
    await recordUsage(env, value, id);
    return Response.json(value, { status: response.status });
  }
  return c.json({ error: "no available account" }, 503);
});
app.onError(
  (error, c) => typeof error.getResponse === "function" ? error.getResponse() : c.json({ error: error.message }, 502)
);
var index_default = {
  fetch: (request, env, ctx) => app.fetch(request, env, ctx),
  async scheduled(event, env, ctx) {
    const ids = await env.CONFIG.get("accounts", { type: "json" }) ?? [];
    ctx.waitUntil(
      Promise.allSettled([
        ...ids.map((id) => credential(env, id).maintain()),
        directory(env).pruneCalls()
      ])
    );
  }
};
export {
  Call,
  Credential,
  Pool,
  index_default as default
};
