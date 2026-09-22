(() => {
  const access = __irAccess,
    create = __irCreate,
    snapshot = __irSnapshot;
  const dialects = [
    "anthropic.messages",
    "openai.responses",
    "openai.chat",
    "gemini.generate",
  ];
  const budgets = {
    none: 0,
    auto: -1,
    minimal: 512,
    low: 1024,
    medium: 8192,
    high: 24576,
    xhigh: 32768,
    max: 128000,
  };
  const level = (n) =>
    n === 0
      ? "none"
      : n < 0
        ? "auto"
        : n <= 512
          ? "minimal"
          : n <= 1024
            ? "low"
            : n <= 8192
              ? "medium"
              : n <= 24576
                ? "high"
                : "xhigh";
  const index = (k) =>
    typeof k === "string" && /^(0|[1-9]\d*)$/.test(k) && Number(k) < 4294967295;
  function rawView(id, path = []) {
    const description = access(id, "read", JSON.stringify(path));
    if (description.kind === "missing") return undefined;
    if (description.kind === "scalar") return description.value;
    const array = description.kind === "array",
      target = array ? [] : {};
    const read = () => access(id, "read", JSON.stringify(path));
    return new Proxy(target, {
      get(_, key) {
        if (key === Symbol.iterator && array)
          return Array.prototype[Symbol.iterator];
        if (
          array &&
          [
            "push",
            "pop",
            "shift",
            "unshift",
            "splice",
            "sort",
            "reverse",
            "copyWithin",
            "fill",
          ].includes(key)
        )
          return function (...args) {
            const value = JSON.parse(JSON.stringify(rawView(id, path)));
            const result = Array.prototype[key].apply(value, args);
            access(id, "set", JSON.stringify(path), JSON.stringify(value));
            return result === value ? this : result;
          };
        if (typeof key === "symbol") return undefined;
        if (array && key !== "length" && !index(key))
          return Reflect.get(Array.prototype, key);
        const value = rawView(id, [...path, key]);
        if (value !== undefined) return value;
        return Reflect.get(array ? Array.prototype : Object.prototype, key);
      },
      set(_, key, value) {
        if (
          typeof key === "symbol" ||
          (array && key !== "length" && !index(key))
        )
          throw new TypeError("IR only supports JSON keys");
        const encoded = JSON.stringify(value);
        if (encoded === undefined)
          access(id, "delete", JSON.stringify([...path, key]));
        else access(id, "set", JSON.stringify([...path, key]), encoded);
        return true;
      },
      deleteProperty(_, key) {
        access(id, "delete", JSON.stringify([...path, String(key)]));
        return true;
      },
      has(_, key) {
        return (
          (key === "length" && array) ||
          read().keys.includes(String(key)) ||
          key in (array ? Array.prototype : Object.prototype)
        );
      },
      ownKeys() {
        const keys = read().keys;
        keys.sort((a, b) =>
          index(a) && index(b)
            ? Number(a) - Number(b)
            : index(a)
              ? -1
              : index(b)
                ? 1
                : 0,
        );
        return array ? [...keys, "length"] : keys;
      },
      getOwnPropertyDescriptor(_, key) {
        if (key === "length" && array)
          return {
            value: read().length,
            writable: true,
            enumerable: false,
            configurable: false,
          };
        if (!read().keys.includes(String(key))) return undefined;
        return {
          value: rawView(id, [...path, key]),
          writable: true,
          enumerable: true,
          configurable: true,
        };
      },
      defineProperty(_, key, desc) {
        if (
          desc.get ||
          desc.set ||
          desc.configurable === false ||
          desc.writable === false
        )
          throw new TypeError("IR property descriptors are not supported");
        if ("value" in desc) this.set(_, key, desc.value);
        return true;
      },
      preventExtensions() {
        throw new TypeError("IR views cannot be frozen");
      },
      setPrototypeOf() {
        throw new TypeError("IR prototype is fixed");
      },
    });
  }
  const get = (o, path) => path.reduce((v, k) => v?.[k], o);
  function put(o, path, value) {
    for (const k of path.slice(0, -1)) {
      if (o[k] == null) o[k] = {};
      o = o[k];
    }
    if (value === undefined) delete o[path.at(-1)];
    else o[path.at(-1)] = value;
  }
  const fields = {
    "anthropic.messages": {
      model: ["model"],
      stream: ["stream"],
      maxTokens: ["max_tokens"],
      temperature: ["temperature"],
      topP: ["top_p"],
      topK: ["top_k"],
      stop: ["stop_sequences"],
      user: ["metadata", "user_id"],
      parallelToolCalls: ["tool_choice", "disable_parallel_tool_use"],
    },
    "openai.responses": {
      model: ["model"],
      stream: ["stream"],
      maxTokens: ["max_output_tokens"],
      temperature: ["temperature"],
      topP: ["top_p"],
      user: ["user"],
      parallelToolCalls: ["parallel_tool_calls"],
    },
    "openai.chat": {
      model: ["model"],
      stream: ["stream"],
      maxTokens: ["max_completion_tokens"],
      temperature: ["temperature"],
      topP: ["top_p"],
      stop: ["stop"],
      user: ["user"],
      parallelToolCalls: ["parallel_tool_calls"],
    },
    "gemini.generate": {
      model: ["model"],
      stream: ["stream"],
      maxTokens: ["generationConfig", "maxOutputTokens"],
      temperature: ["generationConfig", "temperature"],
      topP: ["generationConfig", "topP"],
      topK: ["generationConfig", "topK"],
      stop: ["generationConfig", "stopSequences"],
    },
  };
  function mapped(read, write) {
    return new Proxy(
      {},
      {
        get: (_, k) => read()[k],
        set: (_, k, v) => {
          const copy = read();
          copy[k] = v;
          write(copy, k);
          return true;
        },
        ownKeys: () => Object.keys(read()),
        getOwnPropertyDescriptor: (_, k) =>
          k in read()
            ? {
                value: read()[k],
                enumerable: true,
                configurable: true,
                writable: true,
              }
            : undefined,
      },
    );
  }
  function reasoning(raw, d) {
    const read = () => {
      let value = {};
      if (d === "anthropic.messages") {
        value = {
          level: raw.output_config?.effort,
          budgetTokens: raw.thinking?.budget_tokens,
          adaptive: raw.thinking?.type === "adaptive",
        };
        if (raw.thinking?.type === "disabled") value.budgetTokens = 0;
      }
      if (d === "openai.responses")
        value = {
          level: raw.reasoning?.effort,
          summary: raw.reasoning?.summary,
        };
      if (d === "openai.chat") value = { level: raw.reasoning_effort };
      if (d === "gemini.generate")
        value = {
          budgetTokens: raw.generationConfig?.thinkingConfig?.thinkingBudget,
          includeThoughts:
            raw.generationConfig?.thinkingConfig?.includeThoughts,
        };
      if (value.level === undefined && value.budgetTokens !== undefined)
        value.level = level(value.budgetTokens);
      if (value.budgetTokens === undefined && value.level !== undefined)
        value.budgetTokens = budgets[value.level];
      return value;
    };
    return mapped(read, (v, k) => {
      if (d === "anthropic.messages") {
        if (k === "adaptive")
          put(raw, ["thinking", "type"], v.adaptive ? "adaptive" : "enabled");
        else if (k === "level" && v.adaptive)
          put(raw, ["output_config", "effort"], v.level);
        else if (k === "level" || k === "budgetTokens") {
          const budget = k === "level" ? budgets[v.level] : v.budgetTokens;
          if (!Number.isInteger(budget))
            throw new TypeError("invalid thinking budget or level");
          put(raw, ["thinking", "type"], budget === 0 ? "disabled" : "enabled");
          put(
            raw,
            ["thinking", "budget_tokens"],
            budget === 0 ? undefined : budget,
          );
        }
      } else if (d === "openai.responses")
        put(
          raw,
          ["reasoning", k === "summary" ? "summary" : "effort"],
          k === "summary"
            ? v.summary
            : k === "budgetTokens"
              ? level(v.budgetTokens)
              : v.level,
        );
      else if (d === "openai.chat")
        raw.reasoning_effort =
          k === "budgetTokens" ? level(v.budgetTokens) : v.level;
      else
        put(
          raw,
          [
            "generationConfig",
            "thinkingConfig",
            k === "includeThoughts" ? "includeThoughts" : "thinkingBudget",
          ],
          k === "includeThoughts"
            ? v.includeThoughts
            : k === "level"
              ? budgets[v.level]
              : v.budgetTokens,
        );
    });
  }
  const provenance = new WeakMap();
  // Mapping one message at a time keeps traversals from retaining the whole input.
  const block = (b, d) => {
    if (typeof b === "string") return { type: "text", text: b };
    if (
      d === "anthropic.messages" &&
      ![
        "text",
        "image",
        "tool_use",
        "tool_result",
        "thinking",
        "redacted_thinking",
      ].includes(b.type)
    )
      throw new TypeError(`unsupported Anthropic block: ${b.type}`);
    if (
      d.startsWith("openai.") &&
      ![
        "text",
        "input_text",
        "output_text",
        "input_image",
        "image_url",
        "function_call",
        "function_call_output",
        "reasoning",
      ].includes(b.type)
    )
      throw new TypeError(`unsupported OpenAI block: ${b.type}`);
    if (
      d === "gemini.generate" &&
      ![
        "text",
        "inlineData",
        "fileData",
        "functionCall",
        "functionResponse",
      ].some((k) => k in b)
    )
      throw new TypeError("unsupported Gemini part");
    let out;
    if (d === "anthropic.messages")
      out =
        b.type === "image"
          ? {
              type: "image",
              mime: b.source?.media_type,
              data: b.source?.data,
              url: b.source?.url,
            }
          : b.type === "tool_result"
            ? {
                type: "tool_result",
                id: b.tool_use_id,
                output: b.content,
                isError: b.is_error,
              }
            : b.type === "thinking" || b.type === "redacted_thinking"
              ? {
                  type: "thinking",
                  text: b.thinking,
                  signature: b.signature,
                  encryptedContent: b.data,
                }
              : b.type === "tool_use"
                ? { type: "tool_use", id: b.id, name: b.name, input: b.input }
                : { type: "text", text: b.text };
    else if (d === "gemini.generate")
      out = b.inlineData
        ? {
            type: "image",
            mime: b.inlineData.mimeType,
            data: b.inlineData.data,
          }
        : b.fileData
          ? {
              type: "image",
              mime: b.fileData.mimeType,
              url: b.fileData.fileUri,
            }
          : b.functionCall
            ? {
                type: "tool_use",
                id: b.functionCall.id,
                name: b.functionCall.name,
                input: b.functionCall.args,
              }
            : b.functionResponse
              ? {
                  type: "tool_result",
                  id: b.functionResponse.id,
                  name: b.functionResponse.name,
                  output: b.functionResponse.response,
                }
              : b.thought
                ? {
                    type: "thinking",
                    text: b.text,
                    signature: b.thoughtSignature,
                  }
                : { type: "text", text: b.text };
    else
      out =
        b.type === "input_image" || b.type === "image_url"
          ? {
              type: "image",
              url:
                typeof b.image_url === "string"
                  ? b.image_url
                  : b.image_url?.url,
            }
          : b.type === "function_call"
            ? {
                type: "tool_use",
                id: b.call_id,
                name: b.name,
                input:
                  typeof b.arguments === "string"
                    ? JSON.parse(b.arguments)
                    : b.arguments,
              }
            : b.type === "function_call_output"
              ? { type: "tool_result", id: b.call_id, output: b.output }
              : b.type === "reasoning"
                ? {
                    type: "thinking",
                    text: b.summary?.map((x) => x.text).join(""),
                    encryptedContent: b.encrypted_content,
                  }
                : { type: "text", text: b.text };
    if (out.type === "image" && out.url?.startsWith("data:")) {
      const match = /^data:([^;,]+);base64,(.*)$/s.exec(out.url);
      if (!match) throw new TypeError("unsupported image data URL");
      out.mime = match[1];
      out.data = match[2];
      delete out.url;
    }
    if (b.cache_control !== undefined) out.cacheControl = b.cache_control;
    provenance.set(out, { dialect: d, raw: b });
    return out;
  };
  function emitBlock(b, d) {
    let out;
    if (d === "anthropic.messages")
      out =
        b.type === "text"
          ? { type: "text", text: b.text }
          : b.type === "image"
            ? {
                type: "image",
                source: b.data
                  ? { type: "base64", media_type: b.mime, data: b.data }
                  : { type: "url", url: b.url },
              }
            : b.type === "tool_use"
              ? { type: "tool_use", id: b.id, name: b.name, input: b.input }
              : b.type === "tool_result"
                ? {
                    type: "tool_result",
                    tool_use_id: b.id,
                    content: b.output,
                    is_error: b.isError,
                  }
                : b.encryptedContent
                  ? { type: "redacted_thinking", data: b.encryptedContent }
                  : {
                      type: "thinking",
                      thinking: b.text,
                      signature: b.signature,
                    };
    else if (d === "gemini.generate")
      out =
        b.type === "text"
          ? { text: b.text }
          : b.type === "image"
            ? b.data
              ? { inlineData: { mimeType: b.mime, data: b.data } }
              : { fileData: { mimeType: b.mime, fileUri: b.url } }
            : b.type === "tool_use"
              ? { functionCall: { id: b.id, name: b.name, args: b.input } }
              : b.type === "tool_result"
                ? {
                    functionResponse: {
                      id: b.id,
                      name: b.name ?? b.id,
                      response: b.output,
                    },
                  }
                : {
                    text: b.text,
                    thought: true,
                    thoughtSignature: b.signature,
                  };
    else
      out =
        b.type === "text"
          ? { type: d === "openai.chat" ? "text" : "input_text", text: b.text }
          : b.type === "image"
            ? {
                type: d === "openai.chat" ? "image_url" : "input_image",
                image_url:
                  d === "openai.chat"
                    ? { url: b.url ?? `data:${b.mime};base64,${b.data}` }
                    : (b.url ?? `data:${b.mime};base64,${b.data}`),
              }
            : b.type === "tool_use"
              ? {
                  type: "function_call",
                  call_id: b.id,
                  name: b.name,
                  arguments: JSON.stringify(b.input),
                }
              : b.type === "tool_result"
                ? {
                    type: "function_call_output",
                    call_id: b.id,
                    output:
                      typeof b.output === "string"
                        ? b.output
                        : JSON.stringify(b.output),
                  }
                : {
                    type: "reasoning",
                    summary: b.text
                      ? [{ type: "summary_text", text: b.text }]
                      : [],
                    encrypted_content: b.encryptedContent,
                  };
    if (b.cacheControl !== undefined && d === "anthropic.messages")
      out.cache_control = b.cacheControl;
    const source = provenance.get(b);
    return source?.dialect === d ? { ...source.raw, ...out } : out;
  }
  function decodeMessage(m, d) {
    if (d === "gemini.generate")
      return {
        role: m.role === "model" ? "assistant" : (m.role ?? "user"),
        content: (m.parts ?? []).map((b) => block(b, d)),
      };
    if (
      d === "openai.responses" &&
      ["function_call", "function_call_output", "reasoning"].includes(m.type)
    )
      return {
        role: m.type === "function_call_output" ? "tool" : "assistant",
        content: [block(m, d)],
      };
    const content =
      typeof m.content === "string"
        ? [{ type: "text", text: m.content }]
        : (m.content ?? []).map((b) => block(b, d));
    if (d === "openai.chat" && m.role === "tool")
      return {
        role: "tool",
        content: [
          { type: "tool_result", id: m.tool_call_id, output: m.content },
        ],
      };
    if (m.reasoning_content)
      content.push({ type: "thinking", text: m.reasoning_content });
    for (const t of m.tool_calls ?? [])
      content.push({
        type: "tool_use",
        id: t.id,
        name: t.function.name,
        input: JSON.parse(t.function.arguments),
      });
    return { role: m.role ?? "user", content };
  }
  function encodeMessage(m, d) {
    if (d === "gemini.generate")
      return {
        role: m.role === "assistant" ? "model" : "user",
        parts: m.content.map((b) => emitBlock(b, d)),
      };
    if (d === "openai.chat") {
      if (m.role === "tool") {
        const b = m.content[0];
        return {
          role: "tool",
          tool_call_id: b.id,
          content:
            typeof b.output === "string" ? b.output : JSON.stringify(b.output),
        };
      }
      const calls = m.content
        .filter((b) => b.type === "tool_use")
        .map((b) => ({
          id: b.id,
          type: "function",
          function: { name: b.name, arguments: JSON.stringify(b.input) },
        }));
      return {
        role: m.role,
        content: m.content
          .filter((b) => b.type !== "tool_use")
          .map((b) => emitBlock(b, d)),
        ...(calls.length ? { tool_calls: calls } : {}),
      };
    }
    return { role: m.role, content: m.content.map((b) => emitBlock(b, d)) };
  }
  function encodeMessages(messages, d) {
    if (d !== "openai.responses")
      return messages.map((m) => encodeMessage(m, d));
    return messages.flatMap((m) => {
      const rows = [];
      let content = [];
      const flush = () => {
        if (content.length) {
          rows.push({ role: m.role === "tool" ? "user" : m.role, content });
          content = [];
        }
      };
      for (const b of m.content) {
        if (["tool_use", "tool_result", "thinking"].includes(b.type)) {
          flush();
          rows.push(emitBlock(b, d));
        } else
          content.push({
            ...emitBlock(b, d),
            ...(b.type === "text" && m.role === "assistant"
              ? { type: "output_text" }
              : {}),
          });
      }
      flush();
      return rows;
    });
  }
  function tracked(value, write) {
    if (value === null || typeof value !== "object") return value;
    return new Proxy(value, {
      get: (o, k) => tracked(o[k], write),
      set: (o, k, v) => {
        o[k] = v;
        write();
        return true;
      },
      deleteProperty: (o, k) => {
        delete o[k];
        write();
        return true;
      },
    });
  }
  function messageView(raw, d) {
    const key =
      d === "gemini.generate"
        ? "contents"
        : d === "openai.responses"
          ? "input"
          : "messages";
    if (typeof raw[key] === "string") {
      const value = [
        { role: "user", content: [{ type: "text", text: raw[key] }] },
      ];
      return tracked(value, () => {
        raw[key] = encodeMessages(value, d);
      });
    }
    const a = raw[key];
    const positions = () =>
      Array.from({ length: a.length }, (_, i) => i).filter(
        (i) =>
          d !== "openai.chat" || !["system", "developer"].includes(a[i]?.role),
      );
    const length = () => (d === "openai.chat" ? positions().length : a.length);
    const at = (k) =>
      d === "openai.chat"
        ? positions()[Number(k)]
        : Number(k) < a.length
          ? Number(k)
          : undefined;
    const read = (k) => {
      const i = at(k);
      if (i === undefined) return undefined;
      const value = decodeMessage(a[i], d);
      return tracked(value, () => {
        const rows = encodeMessages([value], d);
        if (rows.length === 1) a[i] = { ...a[i], ...rows[0] };
        else a.splice(i, 1, ...rows);
      });
    };
    return new Proxy([], {
      get(_, k) {
        if (k === "length") return length();
        if (index(k)) return read(k);
        if (
          [
            "push",
            "pop",
            "shift",
            "unshift",
            "splice",
            "sort",
            "reverse",
            "copyWithin",
            "fill",
          ].includes(k)
        )
          return function (...args) {
            const messages = positions().map((i) => decodeMessage(a[i], d));
            const result = Array.prototype[k].apply(messages, args);
            const system =
              d === "openai.chat"
                ? Array.from(a).filter((m) =>
                    ["system", "developer"].includes(m.role),
                  )
                : [];
            raw[key] = [...system, ...messages.map((m) => encodeMessage(m, d))];
            return result === messages ? this : result;
          };
        return Reflect.get(Array.prototype, k);
      },
      set(_, k, v) {
        if (k === "length") {
          const ids = positions();
          if (v > ids.length)
            throw new RangeError("messages cannot contain holes");
          for (const i of ids.slice(v).reverse()) a.splice(i, 1);
        } else {
          const i = at(k);
          a.splice(
            i ?? a.length,
            i === undefined ? 0 : 1,
            ...encodeMessages([v], d),
          );
        }
        return true;
      },
      deleteProperty(_, k) {
        const i = at(k);
        if (i !== undefined) a.splice(i, 1);
        return true;
      },
      has: (_, k) => k === "length" || (index(k) && Number(k) < length()),
      ownKeys: () => [...positions().map((_, i) => String(i)), "length"],
      getOwnPropertyDescriptor: (_, k) =>
        k === "length"
          ? {
              value: length(),
              writable: true,
              configurable: false,
              enumerable: false,
            }
          : index(k) && at(k) !== undefined
            ? {
                value: read(k),
                writable: true,
                enumerable: true,
                configurable: true,
              }
            : undefined,
    });
  }
  const states = new WeakMap();
  globalThis.__makeIR = (bodyOf, streamOf) => ({
    from(request, d) {
      if (!dialects.includes(d))
        throw new TypeError("explicit supported dialect required");
      const id = create(bodyOf(request)),
        raw = rawView(id);
      const key =
        d === "gemini.generate"
          ? "contents"
          : d === "openai.responses"
            ? "input"
            : "messages";
      if (
        !(
          Array.isArray(raw[key]) ||
          (d === "openai.responses" && typeof raw.input === "string")
        )
      )
        throw new TypeError("body does not match declared dialect");
      const ir = {};
      states.set(ir, { id, raw, d });
      Object.defineProperties(ir, {
        protocol: { enumerable: true, get: () => d },
        raw: { enumerable: true, value: raw },
        messages: {
          enumerable: true,
          get: () => messageView(raw, d),
          set: (v) => {
            const system =
              d === "openai.chat"
                ? Array.from(raw[key]).filter((m) =>
                    ["system", "developer"].includes(m.role),
                  )
                : [];
            raw[key] = [...system, ...encodeMessages(v, d)];
          },
        },
        reasoning: {
          enumerable: true,
          get: () => reasoning(raw, d),
          set: (v) => {
            const view = reasoning(raw, d);
            for (const k of Object.keys(v)) view[k] = v[k];
          },
        },
      });
      for (const [name, path] of Object.entries(fields[d]))
        Object.defineProperty(ir, name, {
          enumerable: true,
          get: () => {
            const v = get(raw, path);
            if (name === "parallelToolCalls" && d === "anthropic.messages")
              return v === undefined ? undefined : !v;
            if (name === "maxTokens" && d === "openai.chat")
              return v ?? raw.max_tokens;
            return v;
          },
          set: (v) => {
            if (name === "maxTokens" && d === "openai.chat")
              delete raw.max_tokens;
            put(
              raw,
              path,
              name === "parallelToolCalls" && d === "anthropic.messages"
                ? v === undefined
                  ? undefined
                  : !v
                : v,
            );
          },
        });
      Object.defineProperty(ir, "sessionKeys", {
        enumerable: true,
        get: () => ({
          previousResponseId: raw.previous_response_id,
          cachedContent: raw.cachedContent,
          promptCacheKey: raw.prompt_cache_key,
          sessionId: raw.session_id,
          threadId: raw.thread_id ?? raw.conversation_id,
        }),
      });
      return ir;
    },
    to(ir, target) {
      const state = states.get(ir);
      if (!state) throw new TypeError("not an IR view");
      if (target !== undefined && target !== state.d)
        throw new TypeError("unsupported target dialect");
      return streamOf(snapshot(state.id));
    },
  });
  globalThis.__protocolModel = {
    block,
    emitBlock,
    decodeMessage,
    encodeMessage,
    encodeMessages,
    fields,
    dialects,
    budgets,
    level,
    rawView,
    states,
    tracked,
    mapped,
    get,
    put,
  };
})();
