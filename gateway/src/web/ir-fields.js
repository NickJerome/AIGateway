(() => {
  const base = __makeIR,
    {
      states,
      tracked,
      block,
      emitBlock,
      encodeMessage,
      encodeMessages,
      fields,
      dialects,
      budgets,
      level,
      get,
      put,
    } = __protocolModel;
  const define = (ir, key, read, write) =>
    Object.defineProperty(ir, key, {
      enumerable: true,
      get: () => {
        const value = read();
        return tracked(value, () => write(value));
      },
      set: write,
    });
  function system(raw, d) {
    if (d === "anthropic.messages") {
      const s = raw.system;
      return s === undefined
        ? []
        : typeof s === "string"
          ? [{ type: "text", text: s }]
          : s.map((b) => block(b, d));
    }
    if (d === "openai.responses")
      return raw.instructions === undefined
        ? []
        : [{ type: "text", text: raw.instructions }];
    if (d === "gemini.generate")
      return (raw.systemInstruction?.parts ?? []).map((b) => block(b, d));
    return (raw.messages ?? [])
      .filter((m) => m.role === "system" || m.role === "developer")
      .flatMap((m) =>
        typeof m.content === "string"
          ? [{ type: "text", text: m.content }]
          : (m.content ?? []).map((b) => block(b, d)),
      );
  }
  function writeSystem(raw, d, blocks) {
    if (d === "anthropic.messages")
      raw.system = blocks.map((b) => emitBlock(b, d));
    else if (d === "gemini.generate")
      raw.systemInstruction = { parts: blocks.map((b) => emitBlock(b, d)) };
    else {
      if (blocks.some((b) => b.type !== "text"))
        throw new TypeError("target system requires text");
      const text = blocks.map((b) => b.text).join("\n");
      if (d === "openai.responses") raw.instructions = text;
      else
        raw.messages = [
          { role: "system", content: text },
          ...raw.messages.filter(
            (m) => !["system", "developer"].includes(m.role),
          ),
        ];
    }
  }
  function tools(raw, d) {
    let values = raw.tools ?? [];
    if (d === "gemini.generate")
      values = values.flatMap((t) => t.functionDeclarations ?? []);
    return values.map((t) => {
      if (d === "openai.chat") t = t.function;
      return {
        name: t.name,
        description: t.description,
        schema: t.input_schema ?? t.parameters ?? t.parametersJsonSchema,
        ...(t.strict === undefined ? {} : { strict: t.strict }),
      };
    });
  }
  function writeTools(raw, d, values) {
    const list = values.map((t) =>
      d === "anthropic.messages"
        ? { name: t.name, description: t.description, input_schema: t.schema }
        : d === "openai.chat"
          ? {
              type: "function",
              function: {
                name: t.name,
                description: t.description,
                parameters: t.schema,
                strict: t.strict,
              },
            }
          : d === "openai.responses"
            ? {
                type: "function",
                name: t.name,
                description: t.description,
                parameters: t.schema,
                strict: t.strict,
              }
            : {
                name: t.name,
                description: t.description,
                parametersJsonSchema: t.schema,
              },
    );
    raw.tools =
      d === "gemini.generate" ? [{ functionDeclarations: list }] : list;
  }
  function toolChoice(raw, d) {
    if (d === "gemini.generate") {
      const c = raw.toolConfig?.functionCallingConfig;
      if (!c) return undefined;
      return c.allowedFunctionNames?.length === 1
        ? { name: c.allowedFunctionNames[0] }
        : { AUTO: "auto", ANY: "required", NONE: "none" }[c.mode];
    }
    const c = raw.tool_choice;
    if (c === undefined || typeof c === "string") return c;
    if (d === "anthropic.messages")
      return c.type === "tool"
        ? { name: c.name }
        : c.type === "any"
          ? "required"
          : c.type;
    return { name: c.name ?? c.function?.name };
  }
  function writeChoice(raw, d, c) {
    if (d === "gemini.generate")
      put(
        raw,
        ["toolConfig", "functionCallingConfig"],
        typeof c === "object"
          ? { mode: "ANY", allowedFunctionNames: [c.name] }
          : { mode: { auto: "AUTO", required: "ANY", none: "NONE" }[c] },
      );
    else if (d === "anthropic.messages")
      raw.tool_choice = {
        ...raw.tool_choice,
        ...(typeof c === "object"
          ? { type: "tool", name: c.name }
          : { type: c === "required" ? "any" : c }),
      };
    else
      raw.tool_choice =
        typeof c === "object"
          ? d === "openai.chat"
            ? { type: "function", function: { name: c.name } }
            : { type: "function", name: c.name }
          : c;
  }
  function format(raw, d) {
    if (d === "gemini.generate") {
      const c = raw.generationConfig ?? {};
      return c.responseSchema || c.responseJsonSchema
        ? {
            type: "json_schema",
            schema: c.responseJsonSchema ?? c.responseSchema,
          }
        : c.responseMimeType === "application/json"
          ? { type: "json_object" }
          : undefined;
    }
    let v =
      d === "anthropic.messages"
        ? raw.output_config?.format
        : d === "openai.responses"
          ? raw.text?.format
          : raw.response_format;
    if (!v) return undefined;
    if (d === "openai.chat" && v.type === "json_schema")
      return { type: v.type, ...v.json_schema };
    return { ...v };
  }
  function writeFormat(raw, d, v) {
    if (d === "gemini.generate") {
      put(
        raw,
        ["generationConfig", "responseMimeType"],
        v.type === "text" ? "text/plain" : "application/json",
      );
      put(raw, ["generationConfig", "responseJsonSchema"], v.schema);
      delete raw.generationConfig.responseSchema;
    } else
      put(
        raw,
        d === "anthropic.messages"
          ? ["output_config", "format"]
          : d === "openai.responses"
            ? ["text", "format"]
            : ["response_format"],
        d === "openai.chat" && v.type === "json_schema"
          ? {
              type: "json_schema",
              json_schema: {
                name: v.name ?? "response",
                schema: v.schema,
                strict: v.strict,
              },
            }
          : v,
      );
  }
  const allowed = {
    "anthropic.messages": [
      "model",
      "messages",
      "system",
      "max_tokens",
      "stream",
      "temperature",
      "top_p",
      "top_k",
      "stop_sequences",
      "tools",
      "tool_choice",
      "thinking",
      "output_config",
      "metadata",
    ],
    "openai.responses": [
      "model",
      "input",
      "instructions",
      "max_output_tokens",
      "stream",
      "temperature",
      "top_p",
      "tools",
      "tool_choice",
      "reasoning",
      "text",
      "parallel_tool_calls",
      "user",
      "store",
      "include",
      "truncation",
      "service_tier",
      "previous_response_id",
      "prompt_cache_key",
      "session_id",
      "thread_id",
      "conversation_id",
    ],
    "openai.chat": [
      "model",
      "messages",
      "max_tokens",
      "max_completion_tokens",
      "stream",
      "temperature",
      "top_p",
      "stop",
      "tools",
      "tool_choice",
      "reasoning_effort",
      "response_format",
      "parallel_tool_calls",
      "user",
      "stream_options",
      "prompt_cache_key",
      "session_id",
      "thread_id",
    ],
    "gemini.generate": [
      "model",
      "stream",
      "contents",
      "systemInstruction",
      "generationConfig",
      "tools",
      "toolConfig",
      "cachedContent",
    ],
  };
  // The intersection is deliberately explicit: unsupported constraints fail before I/O.
  function schema(value) {
    if (typeof value === "boolean") return;
    if (!value || typeof value !== "object" || Array.isArray(value))
      throw new TypeError("invalid JSON schema");
    const keys = [
      "type",
      "title",
      "description",
      "properties",
      "required",
      "additionalProperties",
      "items",
      "enum",
      "anyOf",
    ];
    for (const key of Object.keys(value))
      if (!keys.includes(key))
        throw new TypeError(`unsupported schema constraint: ${key}`);
    for (const child of Object.values(value.properties ?? {})) schema(child);
    if (value.items !== undefined) schema(value.items);
    if (typeof value.additionalProperties === "object")
      schema(value.additionalProperties);
    for (const child of value.anyOf ?? []) schema(child);
  }
  function only(value, keys, label) {
    if (!value || typeof value !== "object") return;
    for (const key of Object.keys(value))
      if (!keys.includes(key))
        throw new TypeError(`cannot translate ${label}.${key}`);
  }
  function validateRaw(raw, d, target) {
    if (d === "anthropic.messages") {
      only(raw.metadata, ["user_id"], "metadata");
      only(raw.thinking, ["type", "budget_tokens"], "thinking");
      only(raw.output_config, ["effort", "format"], "output_config");
    } else if (d === "gemini.generate") {
      only(
        raw.generationConfig,
        [
          "maxOutputTokens",
          "temperature",
          "topP",
          "topK",
          "stopSequences",
          "thinkingConfig",
          "responseMimeType",
          "responseSchema",
          "responseJsonSchema",
        ],
        "generationConfig",
      );
      only(
        raw.generationConfig?.thinkingConfig,
        ["thinkingBudget", "includeThoughts"],
        "thinkingConfig",
      );
      only(raw.toolConfig, ["functionCallingConfig"], "toolConfig");
      const choice = raw.toolConfig?.functionCallingConfig;
      only(choice, ["mode", "allowedFunctionNames"], "functionCallingConfig");
      if (choice?.allowedFunctionNames?.length > 1)
        throw new TypeError(
          "multiple allowed tool names cannot cross dialects",
        );
    } else {
      only(raw.reasoning, ["effort", "summary"], "reasoning");
      only(raw.text, ["format"], "text");
      only(raw.stream_options, ["include_usage"], "stream_options");
      if (
        raw.store !== undefined ||
        raw.include?.length ||
        raw.service_tier ||
        (raw.truncation && raw.truncation !== "disabled")
      )
        throw new TypeError("provider execution options cannot cross dialects");
    }
    for (const t of raw.tools ?? []) {
      if (d.startsWith("openai.") && t.type !== "function")
        throw new TypeError("only function tools can cross dialects");
      if (d === "gemini.generate") only(t, ["functionDeclarations"], "tools");
    }
  }
  function crossing(ir, target) {
    const { d, raw } = states.get(ir);
    if (!dialects.includes(target))
      throw new TypeError("unsupported target dialect");
    for (const key of Object.keys(raw))
      if (!allowed[d].includes(key))
        throw new TypeError(`cannot translate provider field: ${key}`);
    if (ir.sessionKeys.previousResponseId || ir.sessionKeys.cachedContent)
      throw new TypeError("account-scoped session state cannot cross dialects");
    validateRaw(raw, d, target);
    for (const field of Object.keys(fields[d]))
      if (!(field in fields[target]) && ir[field] !== undefined)
        throw new TypeError(`target cannot represent ${field}`);
    const messages = Array.from(ir.messages).filter(
      (m) => !["system", "developer"].includes(m.role),
    );
    for (const message of messages)
      for (const b of message.content) {
        if (b.signature || b.encryptedContent)
          throw new TypeError("signed thinking cannot cross dialects");
        if (b.type === "thinking" && target === "openai.chat")
          throw new TypeError("Chat cannot represent thinking blocks");
        if (b.type === "image" && target === "gemini.generate" && !b.mime)
          throw new TypeError("Gemini image requires a media type");
      }
    if (target === "gemini.generate" && ir.parallelToolCalls !== undefined)
      throw new TypeError("Gemini has no equivalent parallel tool switch");
    const toolNames = new Map();
    for (const m of messages)
      for (const b of m.content)
        if (b.type === "tool_use") toolNames.set(b.id, b.name);
    for (const m of messages)
      for (const b of m.content)
        if (b.type === "tool_result" && target === "gemini.generate") {
          b.name ??= toolNames.get(b.id);
          if (!b.name)
            throw new TypeError(
              "Gemini tool result requires its function name",
            );
          if (typeof b.output === "string") b.output = { result: b.output };
        }
    const out = {};
    for (const [name, path] of Object.entries(fields[target])) {
      const value = ir[name];
      if (value !== undefined)
        put(
          out,
          path,
          target === "anthropic.messages" && name === "parallelToolCalls"
            ? !value
            : value,
        );
    }
    const encoded = messages.flatMap((m) => {
      if (
        target === "openai.chat" &&
        m.content.some((b) => b.type === "tool_result")
      ) {
        const rows = [];
        let content = [];
        const flush = () => {
          if (content.length) {
            rows.push(encodeMessage({ ...m, content }, target));
            content = [];
          }
        };
        for (const b of m.content) {
          if (b.type === "tool_result") {
            flush();
            rows.push(encodeMessage({ role: "tool", content: [b] }, target));
          } else content.push(b);
        }
        flush();
        return rows;
      }
      if (target === "openai.responses") return encodeMessages([m], target);
      return [
        encodeMessage(
          {
            ...m,
            role:
              target === "anthropic.messages" && m.role === "tool"
                ? "user"
                : m.role,
          },
          target,
        ),
      ];
    });
    out[
      target === "gemini.generate"
        ? "contents"
        : target === "openai.responses"
          ? "input"
          : "messages"
    ] = encoded;
    const sys = system(raw, d);
    if (sys.length) writeSystem(out, target, sys);
    const ts = tools(raw, d);
    for (const tool of ts) {
      schema(tool.schema);
      if (tool.strict !== undefined && !target.startsWith("openai."))
        throw new TypeError("strict tool schema cannot cross to target");
    }
    if (ts.length) writeTools(out, target, ts);
    const choice = toolChoice(raw, d);
    if (choice !== undefined) {
      if (
        typeof choice === "object"
          ? typeof choice?.name !== "string"
          : !["auto", "required", "none"].includes(choice)
      )
        throw new TypeError("unsupported tool choice");
      writeChoice(out, target, choice);
    }
    const f = format(raw, d);
    if (f) {
      if (!["text", "json_object", "json_schema"].includes(f.type))
        throw new TypeError("unsupported structured output format");
      if (target === "anthropic.messages" && f.type !== "json_schema")
        throw new TypeError("Anthropic requires structured output schema");
      if (f.schema !== undefined) schema(f.schema);
      if (f.strict !== undefined && !target.startsWith("openai."))
        throw new TypeError("strict response schema cannot cross to target");
      const format =
        target === "anthropic.messages"
          ? { type: f.type, schema: f.schema }
          : target === "openai.responses" && f.type === "json_schema"
            ? { ...f, name: f.name ?? "response" }
            : f;
      writeFormat(out, target, format);
    }
    const r = ir.reasoning;
    if (r.level !== undefined || r.budgetTokens !== undefined) {
      if (target === "anthropic.messages") {
        if (r.adaptive) {
          out.thinking = { type: "adaptive" };
          put(out, ["output_config", "effort"], r.level);
        } else
          out.thinking =
            r.budgetTokens === 0
              ? { type: "disabled" }
              : {
                  type: "enabled",
                  budget_tokens: r.budgetTokens ?? budgets[r.level],
                };
      } else if (target === "gemini.generate")
        put(out, ["generationConfig", "thinkingConfig"], {
          thinkingBudget: r.budgetTokens ?? budgets[r.level],
          includeThoughts: r.includeThoughts,
        });
      else if (target === "openai.responses")
        out.reasoning = {
          effort: r.level ?? level(r.budgetTokens),
          summary: r.summary,
        };
      else out.reasoning_effort = r.level ?? level(r.budgetTokens);
    }
    return out;
  }
  globalThis.__makeIR = (bodyOf, streamOf) => {
    const api = base(bodyOf, streamOf);
    return {
      from(input, d) {
        const ir = api.from(input, d);
        const { raw } = states.get(ir);
        define(
          ir,
          "system",
          () => system(raw, d),
          (v) => writeSystem(raw, d, v),
        );
        define(
          ir,
          "tools",
          () => tools(raw, d),
          (v) => writeTools(raw, d, v),
        );
        define(
          ir,
          "toolChoice",
          () => toolChoice(raw, d),
          (v) => writeChoice(raw, d, v),
        );
        define(
          ir,
          "responseFormat",
          () => format(raw, d),
          (v) => writeFormat(raw, d, v),
        );
        return ir;
      },
      to(ir, target) {
        const state = states.get(ir);
        if (!state) throw new TypeError("not an IR view");
        return target === undefined || target === state.d
          ? api.to(ir, target)
          : streamOf(
              __nativeBody(
                new TextEncoder().encode(JSON.stringify(crossing(ir, target))),
              ),
            );
      },
    };
  };
  Object.assign(__protocolModel, {
    system,
    writeSystem,
    tools,
    writeTools,
    toolChoice,
    writeChoice,
    format,
    writeFormat,
    crossing,
  });
})();
