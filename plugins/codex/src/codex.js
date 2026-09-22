export class GatewayError extends Error {
  constructor(status, message, code = "invalid_request_error") {
    super(message);
    this.status = status;
    this.code = code;
  }
}
export function clientKey(request, env) {
  const match = /^Bearer ([^\s]+)$/i.exec(
    request.headers.get("authorization") ?? "",
  );
  const keys = env.CLIENT_API_KEYS;
  if (!Array.isArray(keys) || !keys.length)
    throw new GatewayError(
      401,
      "Client API keys are not configured",
      "invalid_api_key",
    );
  const entry = keys.find(
    (k) =>
      k &&
      typeof k.id === "string" &&
      k.id &&
      typeof k.key === "string" &&
      k.key &&
      k.key !== env.ADMIN_TOKEN &&
      k.enabled !== false &&
      k.key === match?.[1],
  );
  if (!entry) throw new GatewayError(401, "Invalid API key", "invalid_api_key");
  return entry;
}
export function prepare(value, env, key, compact = false) {
  if (!value || Array.isArray(value) || typeof value !== "object")
    throw new GatewayError(400, "Expected a JSON object");
  if (typeof value.model !== "string" || !value.model.trim())
    throw new GatewayError(400, "model is required");
  if (
    key.models !== undefined &&
    (!Array.isArray(key.models) || !key.models.includes(value.model))
  )
    throw new GatewayError(
      403,
      "API key cannot access this model",
      "model_not_allowed",
    );
  if (value.previous_response_id || value.conversation)
    throw new GatewayError(
      400,
      "This Codex HTTP adapter requires full input history; previous_response_id/conversation are not supported",
      "unsupported_parameter",
    );
  if (value.background === true)
    throw new GatewayError(
      400,
      "Background responses are not supported",
      "unsupported_parameter",
    );
  if (value.stream !== undefined && typeof value.stream !== "boolean")
    throw new GatewayError(400, "stream must be a boolean");
  if (compact && value.stream === true)
    throw new GatewayError(400, "compact does not support streaming");
  const body = JSON.parse(JSON.stringify(value));
  if (typeof body.input === "string")
    body.input = [
      {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: body.input }],
      },
    ];
  if (!Array.isArray(body.input))
    throw new GatewayError(400, "input must be a string or array");
  for (const item of body.input) {
    if (!item || typeof item !== "object" || Array.isArray(item))
      throw new GatewayError(400, "invalid input item");
    if (item.role === "system") item.role = "developer";
    if (Array.isArray(item.content))
      for (const part of item.content)
        if (part && typeof part === "object")
          delete part.prompt_cache_breakpoint;
  }
  if (env.MODEL_MAP && Object.hasOwn(env.MODEL_MAP, body.model))
    body.model = env.MODEL_MAP[body.model];
  if (typeof body.model !== "string" || !body.model)
    throw new GatewayError(400, "Invalid model mapping");
  body.instructions ??= "";
  if (typeof body.instructions !== "string")
    throw new GatewayError(400, "instructions must be a string");
  body.store = false;
  if (compact) delete body.stream;
  else {
    body.stream = true;
    if (body.include !== undefined && !Array.isArray(body.include))
      throw new GatewayError(400, "include must be an array");
    body.include = [
      ...new Set([...(body.include ?? []), "reasoning.encrypted_content"]),
    ];
  }
  if (body.tools !== undefined && !Array.isArray(body.tools))
    throw new GatewayError(400, "tools must be an array");
  const normalizeTool = (tool) => {
    if (
      tool &&
      ["web_search_preview", "web_search_preview_2025_03_11"].includes(
        tool.type,
      )
    )
      tool.type = "web_search";
  };
  for (const tool of body.tools ?? []) normalizeTool(tool);
  normalizeTool(body.tool_choice);
  for (const tool of body.tool_choice?.tools ?? []) normalizeTool(tool);
  if (body.tools?.length) body.parallel_tool_calls ??= true;
  else delete body.parallel_tool_calls;
  for (const field of [
    "max_tokens",
    "max_output_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "truncation",
    "prompt_cache_options",
    "prompt_cache_retention",
    "user",
    "context_management",
    "previous_response_id",
    "generate",
    "safety_identifier",
    "stream_options",
    "background",
  ])
    delete body[field];
  if (body.service_tier !== "priority") delete body.service_tier;
  return body;
}
export function upstreamHeaders(request, state, env, compact) {
  const headers = new Headers({
    authorization: `Bearer ${state.access}`,
    "chatgpt-account-id": state.accountId,
    "content-type": "application/json",
    accept: compact ? "application/json" : "text/event-stream",
    originator: env.ORIGINATOR ?? "codex_cli_rs",
    "user-agent": env.UPSTREAM_USER_AGENT ?? "aigateway-codex/1.0",
  });
  for (const name of [
    "version",
    "x-codex-beta-features",
    "x-codex-turn-metadata",
    "x-codex-turn-state",
    "x-client-request-id",
    "x-codex-window-id",
    "thread-id",
    "session-id",
    "x-openai-internal-codex-responses-lite",
  ])
    if (request.headers.has(name)) headers.set(name, request.headers.get(name));
  return headers;
}
export function responseHeaders(upstream, stream) {
  const headers = new Headers({
    "content-type": stream ? "text/event-stream" : "application/json",
    "cache-control": "no-store",
  });
  for (const name of ["x-request-id", "x-codex-turn-state", "retry-after"])
    if (upstream.headers.has(name))
      headers.set(name, upstream.headers.get(name));
  return headers;
}
export function retrySeconds(response) {
  const value = response.headers.get("retry-after");
  const seconds =
    value && /^\d+$/.test(value)
      ? Number(value)
      : value
        ? (Date.parse(value) - Date.now()) / 1000
        : 60;
  return Number.isFinite(seconds)
    ? Math.min(3600, Math.max(1, Math.ceil(seconds)))
    : 60;
}
