import { Hono } from "hono/quick";
import { credential, directory, jwt } from "./auth.js";
import {
  GatewayError,
  clientKey,
  prepare,
  upstreamHeaders,
  responseHeaders,
  retrySeconds,
} from "./codex.js";
import { aggregate, stream } from "./stream.js";
export { Credential, Pool } from "./auth.js";
const app = new Hono();
const fail = (c, status, message, code) =>
  c.json(
    {
      error: { message, type: code ?? "api_error", code: code ?? "api_error" },
    },
    status,
  );
app.use("/codex/admin/*", async (c, next) => {
  if (
    !c.env.ADMIN_TOKEN ||
    c.req.header("authorization") !== `Bearer ${c.env.ADMIN_TOKEN}`
  )
    return fail(c, 401, "Invalid administrator token", "invalid_api_key");
  await next();
});
app.get("/codex/admin/accounts", async (c) => {
  const ids = (await c.env.CONFIG.get("accounts", { type: "json" })) ?? [];
  return c.json(
    await Promise.all(ids.map((id) => credential(c.env, id).status())),
  );
});
app.post("/codex/admin/accounts/import", async (c) => {
  const value = await c.req.json();
  const tokens = value.tokens ?? value;
  const access = tokens.access_token;
  const claims = jwt(
    tokens.id_token ??
      (typeof access === "string" && access.split(".").length === 3
        ? access
        : undefined),
  );
  const id =
    tokens.account_id ??
    value.account_id ??
    claims["https://api.openai.com/auth"]?.chatgpt_account_id;
  if (
    typeof access !== "string" ||
    !access ||
    typeof id !== "string" ||
    !id ||
    id.length > 256
  )
    throw new GatewayError(
      400,
      "access_token and account_id (or token claims) are required",
    );
  const expiresAt =
    value.expires_at !== undefined
      ? Number(value.expires_at)
      : value.expired
        ? Date.parse(value.expired)
        : claims.exp
          ? claims.exp * 1000
          : Date.now() + 3600000;
  if (!Number.isFinite(expiresAt) || expiresAt < 0)
    throw new GatewayError(400, "Invalid expiry");
  if (
    tokens.refresh_token !== undefined &&
    typeof tokens.refresh_token !== "string"
  )
    throw new GatewayError(400, "Invalid refresh token");
  const proxy = value.proxy ?? "direct";
  validateProxy(proxy);
  const result = await credential(c.env, id).importState({
    id,
    accountId: id,
    access,
    refresh: tokens.refresh_token,
    expiresAt,
    proxy,
    state: "active",
    enabled: true,
    cooldown: 0,
  });
  await directory(c.env).register(id);
  return c.json(result, 201);
});
app.put("/codex/admin/accounts/:id/enabled", async (c) => {
  const value = await c.req.json();
  if (typeof value.enabled !== "boolean")
    throw new GatewayError(400, "enabled must be a boolean");
  return c.json(
    await credential(c.env, c.req.param("id")).setEnabled(value.enabled),
  );
});
function validateProxy(proxy) {
  if (
    typeof proxy !== "string" ||
    (proxy !== "direct" && !/^(https?|socks5h?):\/\//.test(proxy))
  )
    throw new GatewayError(400, "Unsupported proxy URL");
}
app.post("/codex/admin/accounts/login", async (c) => {
  const proxy = c.req.query("proxy") ?? "direct";
  validateProxy(proxy);
  return c.json(await directory(c.env).login(proxy));
});
app.post("/codex/admin/accounts/login/:id/poll", async (c) => {
  const result = await directory(c.env).poll(c.req.param("id"));
  return c.json(result, result.state === "expired" ? 410 : 200);
});
app.get("/codex/admin/usage", async (c) =>
  c.json(
    await c.env.USAGE.query({
      since: Number(c.req.query("since") ?? 0),
      groupBy: ["credential", "model"],
    }),
  ),
);
for (const path of ["/v1/models", "/codex/models"])
  app.get(path, (c) => {
    const key = clientKey(c.req.raw, c.env);
    const models = c.env.MODELS ?? Object.keys(c.env.MODEL_MAP ?? {});
    return c.json({
      object: "list",
      data: models
        .filter(
          (m) =>
            typeof m === "string" && (!key.models || key.models.includes(m)),
        )
        .map((id) => ({ id, object: "model", owned_by: "codex" })),
    });
  });
async function digest(text) {
  const bytes = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return [...new Uint8Array(bytes)]
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("");
}
async function record(env, value, account, caller) {
  if (
    !value?.id ||
    !value.usage ||
    !(await directory(env).claimUsage(account + ":" + value.id))
  )
    return;
  const u = value.usage;
  env.USAGE.record(
    Object.fromEntries(
      Object.entries({
        credential: account,
        caller,
        model: value.model ?? "",
        input: u.input_tokens,
        output: u.output_tokens,
        cached: u.input_tokens_details?.cached_tokens,
        reasoning: u.output_tokens_details?.reasoning_tokens,
      }).filter(([, v]) => v !== undefined),
    ),
  );
}
async function respond(c) {
  const request = c.req.raw,
    env = c.env;
  const key = clientKey(request, env);
  const input = await c.req.json();
  const compact = new URL(request.url).pathname.endsWith("/compact");
  const body = prepare(input, env, key, compact);
  const wantStream = !compact && input.stream === true;
  const session =
    body.prompt_cache_key ??
    request.headers.get("session-id") ??
    request.headers.get("thread-id");
  if (session !== undefined && session !== null && typeof session !== "string")
    throw new GatewayError(400, "session/cache key must be a string");
  const sticky = session
    ? "session:" + (await digest(key.id + "\0" + session))
    : null;
  const preferred = sticky ? await env.CONFIG.get(sticky) : null;
  const candidates = await directory(env).candidates(preferred);
  const base = (
    env.UPSTREAM ?? "https://chatgpt.com/backend-api/codex"
  ).replace(/\/$/, "");
  let lastError;
  for (const id of candidates) {
    if (request.signal.aborted) throw request.signal.reason;
    let state;
    try {
      state = await credential(env, id).token();
    } catch {
      continue;
    }
    let response;
    for (let attempt = 0; attempt < 2; attempt++) {
      response = await c.executionCtx.fetch(
        base + (compact ? "/responses/compact" : "/responses"),
        {
          method: "POST",
          proxy: state.proxy ?? "direct",
          headers: upstreamHeaders(request, state, env, compact),
          body: JSON.stringify(body),
          signal: request.signal,
        },
      );
      if (response.status !== 401 || attempt === 1 || !state.refresh) break;
      await response.body?.cancel();
      response = null;
      try {
        state = await credential(env, id).token(true, state.access);
      } catch {
        break;
      }
    }
    if (!response) continue;
    if ([401, 429].includes(response.status)) {
      const seconds = response.status === 429 ? retrySeconds(response) : 30;
      await response.body?.cancel();
      await credential(env, id).cooldown(seconds);
      lastError = { status: response.status, seconds };
      continue;
    }
    if (!response.ok) {
      const headers = responseHeaders(response, false);
      let data;
      try {
        data = await response.json();
      } catch {
        data = {};
      }
      return Response.json(
        {
          error: {
            message:
              data.error?.message ??
              data.detail ??
              `Codex upstream returned ${response.status}`,
            type: data.error?.type ?? "upstream_error",
            code: data.error?.code ?? "upstream_error",
          },
        },
        { status: response.status, headers },
      );
    }
    if (sticky) await env.CONFIG.put(sticky, id, { expirationTtl: 3600 });
    const headers = responseHeaders(response, wantStream);
    if (compact) {
      const value = await response.json();
      if (value.error)
        return fail(
          c,
          502,
          value.error.message ?? "Compaction failed",
          "upstream_error",
        );
      await record(env, value, id, key.id);
      return Response.json(value, { headers });
    }
    if (!response.headers.get("content-type")?.includes("text/event-stream")) {
      await response.body?.cancel();
      return fail(
        c,
        502,
        "Codex upstream did not return SSE",
        "upstream_error",
      );
    }
    const onResult = (value) => record(env, value, id, key.id);
    if (wantStream)
      return new Response(stream(response.body, onResult), { headers });
    return Response.json(await aggregate(response.body, onResult), { headers });
  }
  if (lastError?.status === 429)
    c.header("retry-after", String(lastError.seconds));
  return fail(
    c,
    lastError?.status === 429 ? 429 : 503,
    "No available Codex account",
    lastError?.status === 429 ? "rate_limit_error" : "account_unavailable",
  );
}
for (const path of [
  "/v1/responses",
  "/v1/responses/compact",
  "/codex/responses",
  "/codex/responses/compact",
])
  app.post(path, respond);
app.notFound((c) => fail(c, 404, "Unknown Codex route", "not_found"));
app.onError((error, c) =>
  fail(
    c,
    error.status ?? (error instanceof SyntaxError ? 400 : 502),
    error.message,
    error.code,
  ),
);
export default {
  fetch: (r, env, ctx) => app.fetch(r, env, ctx),
  async scheduled(event, env, ctx) {
    const ids = (await env.CONFIG.get("accounts", { type: "json" })) ?? [];
    ctx.waitUntil(
      Promise.allSettled([
        ...ids.map((id) => credential(env, id).maintain()),
        directory(env).pruneLogins(),
      ]),
    );
  },
};
