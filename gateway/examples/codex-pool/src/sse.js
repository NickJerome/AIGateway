import { directory } from "./auth.js";
export async function remember(env, id, acct) {
  if (id) await env.CONFIG.put("response:" + id, acct, { expirationTtl: 3600 });
}
export async function recordUsage(env, value, acct) {
  if (!value?.id || !value.usage) return;
  if (!(await directory(env).claimUsage(value.id))) return;
  const u = value.usage;
  const record = {
    credential: acct,
    model: value.model ?? "",
    input: u.input_tokens,
    output: u.output_tokens,
    cached: u.input_tokens_details?.cached_tokens,
    reasoning: u.output_tokens_details?.reasoning_tokens,
  };
  env.USAGE.record(
    Object.fromEntries(
      Object.entries(record).filter(([, v]) => v !== undefined),
    ),
  );
}
export function inspectSSE(body, env, acct) {
  let pending = "";
  async function inspect(frame) {
    const text = frame
      .split(/\r?\n/)
      .filter((l) => l.startsWith("data:"))
      .map((l) => l.slice(5).replace(/^ /, ""))
      .join("\n");
    if (text && text !== "[DONE]") {
      let value;
      try {
        value = JSON.parse(text);
      } catch {}
      if (value?.response?.id) await remember(env, value.response.id, acct);
      if (value?.type === "response.completed")
        await recordUsage(env, value.response, acct);
    }
  }
  return body
    .pipeThrough(new TextDecoderStream())
    .pipeThrough(
      new TransformStream({
        async transform(chunk, c) {
          pending += chunk;
          for (;;) {
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
        },
      }),
    )
    .pipeThrough(new TextEncoderStream());
}
