const paths = new Set(["/v1/chat/completions", "/v1/responses", "/v1/messages"]);
const fail = (status, message) => Response.json({
  error: { type: "invalid_request_error", message },
}, { status });

// Observe the response in the forwarding stream; bytes and backpressure stay unchanged.
function usageStream(env, protocol, streaming, model) {
  const decoder = new TextDecoder();
  let buffer = "", data = "", eventName = "", usage = {}, reported = false;
  let disabled = false, terminal = false;
  const number = (value) => Number.isSafeInteger(value) && value >= 0;
  function report() {
    if (reported || ![usage.input_tokens, usage.prompt_tokens, usage.output_tokens, usage.completion_tokens].some(number)) return;
    reported = true;
    const input = usage.input_tokens ?? usage.prompt_tokens;
    const output = usage.output_tokens ?? usage.completion_tokens;
    const cached = usage.cache_read_input_tokens ?? usage.input_tokens_details?.cached_tokens
      ?? usage.prompt_tokens_details?.cached_tokens;
    const written = usage.cache_creation_input_tokens;
    const reasoning = usage.output_tokens_details?.reasoning_tokens
      ?? usage.completion_tokens_details?.reasoning_tokens;
    const record = { model, protocol, stream: streaming, complete: !streaming || terminal,
      user_id: env.IDENTITY?.userId ?? null, key_id: env.IDENTITY?.keyId ?? null };
    // Anthropic reports uncached input separately; expose total input consistently.
    if (number(input)) record.input_tokens = input + (protocol === "messages"
      ? (number(cached) ? cached : 0) + (number(written) ? written : 0) : 0);
    if (number(output)) record.output_tokens = output;
    if (number(cached)) record.cache_read_tokens = cached;
    if (number(written)) record.cache_write_tokens = written;
    if (number(reasoning)) record.reasoning_tokens = reasoning;
    if (number(usage.total_tokens)) record.total_tokens = usage.total_tokens;
    else if (number(record.input_tokens) && number(output)) record.total_tokens = record.input_tokens + output;
    try { env.USAGE.record(record); }
    catch { console.warn("Octopus usage could not be recorded"); }
  }
  function observe(value) {
    const source = value.response ?? value.message ?? value;
    if (source.usage && typeof source.usage === "object") usage = { ...usage, ...source.usage };
    if (typeof source.model === "string") model = source.model;
    const type = value.type || eventName;
    if (type === "response.completed" || type === "message_stop") {
      terminal = true;
      report();
    }
  }
  function event() {
    if (data.trim() === "[DONE]") { terminal = true; report(); }
    else if (data) { try { observe(JSON.parse(data)); } catch { /* Non-JSON SSE data is forwarded unchanged. */ } }
    data = "";
    eventName = "";
  }
  function parse(final = false) {
    if (!streaming) return;
    let match;
    while ((match = /\r\n|\r|\n/.exec(buffer))) {
      if (!final && match[0] === "\r" && match.index + 1 === buffer.length) break;
      const line = buffer.slice(0, match.index);
      buffer = buffer.slice(match.index + match[0].length);
      if (!line) event();
      else if (line.startsWith("data:")) data += (data ? "\n" : "") + line.slice(5).replace(/^ /, "");
      else if (line.startsWith("event:")) eventName = line.slice(6).trim();
      if (data.length > 1024 * 1024) { disable(); break; }
    }
  }
  function disable() {
    disabled = true;
    buffer = data = "";
    console.warn("Octopus usage parsing skipped: response JSON or SSE event exceeds 1 MiB");
  }
  return new TransformStream({
    transform(chunk, controller) {
      controller.enqueue(chunk);
      if (disabled) return;
      buffer += decoder.decode(chunk, { stream: true });
      parse();
      if (buffer.length + data.length > 1024 * 1024) disable();
    },
    flush() {
      if (!disabled) {
        buffer += decoder.decode();
        if (streaming) { buffer += "\n\n"; parse(true); }
        else { try { observe(JSON.parse(buffer)); } catch { /* Keep invalid upstream payload unchanged. */ } }
      }
      report();
      if (!reported) console.warn("Octopus response contained no reportable usage");
    },
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname.replace(/^\/octopus(?=\/)/, "");
    const model = env.MODEL || "qwen3.8-flash-next";
    if (path === "/v1/models") {
      if (request.method !== "GET") return fail(405, "Use GET for models");
      return Response.json({ object: "list", data: [
        { id: model, object: "model", created: 0, owned_by: "octopus" },
      ] });
    }
    if (!paths.has(path)) return fail(404, "Unknown endpoint");
    if (request.method !== "POST") return fail(405, "Use POST for inference");
    if (!env.API_KEY) return fail(503, "Octopus API_KEY is not configured");
    let body;
    try { body = await request.json(); }
    catch { return fail(400, "Request body must be JSON"); }
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return fail(400, "Request body must be a JSON object");
    }
    body.model ??= model;
    if (body.model !== model) return fail(400, `Supported model: ${model}`);

    const protocol = path.slice("/v1/".length);
    if (protocol === "chat/completions" && body.stream === true) {
      body.stream_options = { ...body.stream_options, include_usage: true };
    }
    // Only protocol headers cross the upstream boundary; never forward client credentials.
    const headers = new Headers({
      "content-type": "application/json",
      authorization: `Bearer ${env.API_KEY}`,
    });
    if (path === "/v1/messages") {
      headers.set("x-api-key", env.API_KEY);
      headers.set("anthropic-version", request.headers.get("anthropic-version") || "2023-06-01");
      const beta = request.headers.get("anthropic-beta");
      if (beta) headers.set("anthropic-beta", beta);
    }
    const accept = request.headers.get("accept");
    if (accept) headers.set("accept", accept);
    let response;
    try {
      response = await fetch(`${(env.UPSTREAM || "https://octopus.wangjerome.com").replace(/\/$/, "")}${path}${url.search}`, {
        method: "POST", headers, body: JSON.stringify(body),
        signal: request.signal, redirect: "manual",
      });
    } catch {
      console.error(`Octopus ${protocol}: upstream connection failed`);
      return fail(502, "Could not reach Octopus upstream");
    }
    console.info(`Octopus ${protocol}: upstream HTTP ${response.status}`);
    if (response.status >= 300 && response.status < 400) {
      await response.body?.cancel();
      return fail(502, "Unexpected upstream redirect");
    }
    const outputHeaders = new Headers();
    for (const name of ["content-type", "retry-after", "x-request-id", "request-id"]) {
      const value = response.headers.get(name);
      if (value) outputHeaders.set(name, value);
    }
    outputHeaders.set("cache-control", "no-store");
    outputHeaders.set("x-accel-buffering", "no");
    // Keep JSON/errors/SSE unchanged; do not buffer a streamed model response.
    const streaming = (response.headers.get("content-type") || "").includes("text/event-stream");
    const output = response.ok && response.body
      ? response.body.pipeThrough(usageStream(env, protocol, streaming, model)) : response.body;
    return new Response(output, { status: response.status, headers: outputHeaders });
  },
};
