import { registerRealtime } from "./realtime.js";
export { Call } from "./realtime.js";
import { Hono } from "hono/quick";
import { bearerAuth } from "hono/bearer-auth";
import { credential, directory } from "./auth.js";
import { remember, recordUsage, inspectSSE } from "./sse.js";
export { Credential, Pool } from "./auth.js";
const app = new Hono();
registerRealtime(app);
app.use("/codex/admin/*", async (c, next) => {
  if (!c.env.ADMIN_TOKEN)
    return c.json({ error: "administrator token is not configured" }, 401);
  return bearerAuth({ token: c.env.ADMIN_TOKEN })(c, next);
});
app.get("/codex/admin/accounts", async (c) => {
  const ids = (await c.env.CONFIG.get("accounts", { type: "json" })) ?? [];
  return c.json(
    await Promise.all(ids.map((id) => credential(c.env, id).status())),
  );
});
app.get("/codex/admin/usage", async (c) =>
  c.json(
    await c.env.USAGE.query({
      since: Number(c.req.query("since") ?? 0),
      groupBy: ["credential", "model"],
    }),
  ),
);
app.post("/codex/admin/accounts/login", async (c) =>
  c.json(await directory(c.env).login(c.req.query("proxy") ?? "direct")),
);
app.post("/codex/admin/accounts/login/:id/poll", async (c) => {
  const result = await directory(c.env).poll(c.req.param("id"));
  return c.json(result, result.state === "expired" ? 410 : 200);
});
app.post("/codex/responses", async (c) => {
  const request = c.req.raw,
    env = c.env,
    ir = env.IR.from(request, "openai.responses");
  const ids = (await env.CONFIG.get("accounts", { type: "json" })) ?? [];
  const hard = ir.sessionKeys.previousResponseId;
  const soft =
    ir.sessionKeys.promptCacheKey ??
    ir.sessionKeys.sessionId ??
    ir.sessionKeys.threadId;
  const mapped = hard
    ? await env.CONFIG.get("response:" + hard)
    : soft
      ? await env.CONFIG.get("soft:" + soft)
      : null;
  if (hard && !mapped)
    return c.json(
      { error: "previous_response_id has no account mapping" },
      409,
    );
  const candidates = hard
    ? [mapped]
    : [...(mapped ? [mapped] : []), ...ids.filter((id) => id !== mapped)];
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
        accept: "text/event-stream",
      },
      body: env.IR.to(ir),
      signal: request.signal,
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
          fallback: 1,
        });
    }
    if (response.headers.get("content-type")?.includes("text/event-stream"))
      return new Response(inspectSSE(response.body, env, id), {
        status: response.status,
        headers: response.headers,
      });
    const value = await response.json();
    await remember(env, value.id, id);
    await recordUsage(env, value, id);
    return Response.json(value, { status: response.status });
  }
  return c.json({ error: "no available account" }, 503);
});
app.onError((error, c) =>
  typeof error.getResponse === "function"
    ? error.getResponse()
    : c.json({ error: error.message }, 502),
);
export default {
  fetch: (request, env, ctx) => app.fetch(request, env, ctx),
  async scheduled(event, env, ctx) {
    const ids = (await env.CONFIG.get("accounts", { type: "json" })) ?? [];
    ctx.waitUntil(
      Promise.allSettled([
        ...ids.map((id) => credential(env, id).maintain()),
        directory(env).pruneCalls(),
      ]),
    );
  },
};
