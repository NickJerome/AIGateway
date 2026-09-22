import { DurableObject } from "cloudflare:workers";
import { credential, directory } from "./auth.js";
const call = (env, id) => env.CALLS.get(env.CALLS.idFromName(id));
const valid = (id) => /^[A-Za-z0-9_-]{1,128}$/.test(id);
export class Call extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.state = await ctx.storage.get("call");
    });
  }
  async install(account, expiresAt) {
    if (this.state && this.state.expiresAt > Date.now())
      throw new Error("call already exists");
    const state = { account, expiresAt, lease: null };
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
}
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
    "x-oai-attestation",
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
        request.headers.get("sec-websocket-protocol"),
      );
  } else {
    h.set(
      "content-type",
      request.headers.get("content-type") ?? "application/sdp",
    );
    if (request.headers.has("openai-alpha"))
      h.set("openai-alpha", request.headers.get("openai-alpha"));
  }
  return h;
}
function upgraded(request) {
  return request.headers.get("upgrade")?.toLowerCase() === "websocket";
}
const required = () =>
  new Response("WebSocket upgrade required", {
    status: 426,
    headers: { upgrade: "websocket" },
  });
export function registerRealtime(app) {
  app.get("/codex/realtime", async (c) => {
    if (c.req.query("call_id")) return sideband(c, c.req.query("call_id"));
    if (!upgraded(c.req.raw)) return required();
    const ids = (await c.env.CONFIG.get("accounts", { type: "json" })) ?? [];
    for (const id of ids) {
      let state;
      try {
        state = await credential(c.env, id).token();
      } catch {
        continue;
      }
      const url = new URL(
        c.env.REALTIME_UPSTREAM ?? "https://api.openai.com/v1/realtime",
      );
      url.searchParams.set("model", c.req.query("model") ?? "gpt-realtime");
      const response = await c.executionCtx.fetch(url.href, {
        proxy: state.proxy ?? "direct",
        headers: headers(c.req.raw, state, true),
        signal: c.req.raw.signal,
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
        headers: response.headers,
      });
    }
    return c.json({ error: "no available account" }, 503);
  });
  app.post("/codex/realtime/calls", async (c) => {
    const ids = (await c.env.CONFIG.get("accounts", { type: "json" })) ?? [];
    const body = await c.req.raw.arrayBuffer();
    for (const id of ids) {
      let state;
      try {
        state = await credential(c.env, id).token();
      } catch {
        continue;
      }
      const response = await c.executionCtx.fetch(
        c.env.REALTIME_BOOTSTRAP ??
          "https://chatgpt.com/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas",
        {
          method: "POST",
          headers: headers(c.req.raw, state, false),
          body,
          proxy: state.proxy ?? "direct",
          signal: c.req.raw.signal,
        },
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
        c.env.REALTIME_BOOTSTRAP ?? "https://chatgpt.com",
      );
      const idCall = upstream.pathname.split("/").filter(Boolean).at(-1);
      if (!valid(idCall)) throw new Error("invalid upstream call ID");
      await call(c.env, idCall).install(
        id,
        Date.now() + Number(c.env.REALTIME_CALL_TTL_MS ?? 3600000),
      );
      await directory(c.env).registerCall(idCall);
      const output = new Headers(response.headers);
      output.set(
        "location",
        new URL("/codex/realtime/calls/" + idCall, c.req.url).href,
      );
      return new Response(response.body, {
        status: response.status,
        headers: output,
      });
    }
    return c.json({ error: "no available account" }, 503);
  });
  app.get("/codex/realtime/calls/:id", (c) => sideband(c, c.req.param("id")));
}
async function sideband(c, id) {
  if (!upgraded(c.req.raw)) return required();
  if (!valid(id)) return c.json({ error: "invalid call ID" }, 400);
  const object = call(c.env, id),
    claim = await object.claim();
  if (claim.error)
    return c.json(
      { error: claim.error },
      claim.error === "occupied" ? 409 : 404,
    );
  try {
    const state = await credential(c.env, claim.account).token();
    const url = new URL(
      c.env.REALTIME_UPSTREAM ?? "https://api.openai.com/v1/realtime",
    );
    url.searchParams.set("intent", "quicksilver");
    url.searchParams.set("call_id", id);
    const response = await c.executionCtx.fetch(url.href, {
      headers: headers(c.req.raw, state, true),
      proxy: state.proxy ?? "direct",
      signal: c.req.raw.signal,
    });
    if (response.status !== 101) {
      await object.done(claim.lease);
      return response;
    }
    const pair = new WebSocketPair(),
      down = pair[1],
      up = response.webSocket;
    down.accept();
    up.accept();
    let ended = false;
    const done = (source, destination, event) => {
      if (ended) return;
      ended = true;
      clearTimeout(timer);
      try {
        destination.close(
          event.code === 1006 ? 1011 : event.code || 1000,
          event.reason ?? "",
        );
      } catch {}
      c.executionCtx.waitUntil(object.done(claim.lease));
    };
    down.addEventListener("message", (event) => up.send(event.data));
    up.addEventListener("message", (event) => down.send(event.data));
    down.addEventListener("close", (event) => done(down, up, event));
    up.addEventListener("close", (event) => done(up, down, event));
    const timer = setTimeout(
      () => {
        down.close(1000, "call expired");
        up.close(1000, "call expired");
        c.executionCtx.waitUntil(object.done(claim.lease));
      },
      Math.max(0, claim.expiresAt - Date.now()),
    );
    const output = new Headers();
    if (response.headers.has("sec-websocket-protocol"))
      output.set(
        "sec-websocket-protocol",
        response.headers.get("sec-websocket-protocol"),
      );
    return new Response(null, {
      status: 101,
      webSocket: pair[0],
      headers: output,
    });
  } catch (error) {
    await object.done(claim.lease);
    throw error;
  }
}
