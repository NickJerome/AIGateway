import { GatewayError } from "./codex.js";
const encoder = new TextEncoder();
const MAX_FRAME = 1 << 20;
const MAX_RESULT = 8 << 20;
export async function* frames(body, cancellation = {}) {
  if (!body)
    throw new GatewayError(
      502,
      "Upstream response has no body",
      "upstream_error",
    );
  const reader = body.getReader(),
    decoder = new TextDecoder();
  cancellation.cancel = () => reader.cancel();
  let pending = "";
  const parse = (raw) => {
    const data = raw
      .replace(/\r\n/g, "\n")
      .split("\n")
      .filter((l) => l.startsWith("data:"))
      .map((l) => l.slice(5).replace(/^ /, ""))
      .join("\n");
    if (!data || data === "[DONE]") return { raw, value: null };
    return { raw, value: JSON.parse(data) };
  };
  try {
    for (;;) {
      const { value, done } = await reader.read();
      pending += decoder.decode(value, { stream: !done });
      for (;;) {
        const end = /\r?\n\r?\n/.exec(pending);
        if (!end) break;
        const raw = pending.slice(0, end.index + end[0].length);
        pending = pending.slice(raw.length);
        if (encoder.encode(raw).length > MAX_FRAME)
          throw new Error("SSE frame exceeds 1 MiB");
        yield parse(raw);
      }
      if (encoder.encode(pending).length > MAX_FRAME)
        throw new Error("SSE frame exceeds 1 MiB");
      if (done) {
        if (pending.trim()) yield parse(pending + "\n\n");
        return;
      }
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
export function terminal(value) {
  return [
    "response.completed",
    "response.incomplete",
    "response.failed",
    "response.cancelled",
    "error",
  ].includes(value?.type);
}
function failure(value) {
  if (
    ["error", "response.failed", "response.cancelled"].includes(value?.type) ||
    ["failed", "cancelled", "canceled"].includes(value?.response?.status) ||
    value?.response?.error
  )
    return (
      value.error?.message ??
      value.response?.error?.message ??
      "Codex generation failed"
    );
}
export async function aggregate(body, onResult) {
  const items = new Map();
  let size = 0;
  for await (const { value } of frames(body)) {
    if (!value) continue;
    const error = failure(value);
    if (error) throw new GatewayError(502, error, "upstream_error");
    if (value.type === "response.output_item.done" && value.item) {
      const index = value.output_index ?? items.size;
      size +=
        encoder.encode(JSON.stringify(value.item)).length -
        (items.has(index)
          ? encoder.encode(JSON.stringify(items.get(index))).length
          : 0);
      if (size > MAX_RESULT)
        throw new GatewayError(
          502,
          "Aggregated response exceeds 8 MiB",
          "upstream_error",
        );
      items.set(index, value.item);
    }
    if (terminal(value)) {
      const result = value.response;
      if (!result || !["completed", "incomplete"].includes(result.status))
        throw new GatewayError(
          502,
          "Invalid Codex terminal response",
          "upstream_error",
        );
      if (!result.output?.length && items.size)
        result.output = [...items]
          .sort(([a], [b]) => a - b)
          .map(([, item]) => item);
      await onResult(result);
      return result;
    }
  }
  throw new GatewayError(
    502,
    "Upstream ended before a Responses terminal event",
    "upstream_error",
  );
}
export function stream(body, onResult) {
  const cancellation = {};
  const source = frames(body, cancellation);
  let ended = false;
  return new ReadableStream({
    async pull(controller) {
      try {
        if (ended) {
          controller.close();
          return;
        }
        const next = await source.next();
        if (next.done)
          throw new Error("Upstream ended before a Responses terminal event");
        const { raw, value } = next.value;
        if (terminal(value)) {
          if (value.response) await onResult(value.response);
          ended = true;
          await source.return();
        }
        controller.enqueue(encoder.encode(raw));
      } catch (error) {
        ended = true;
        await source.return().catch(() => {});
        controller.enqueue(
          encoder.encode(
            `event: error\ndata: ${JSON.stringify({ type: "error", code: "upstream_error", message: error.message })}\n\n`,
          ),
        );
        controller.close();
      }
    },
    async cancel() {
      ended = true;
      await cancellation.cancel?.();
      await source.return();
    },
  });
}
