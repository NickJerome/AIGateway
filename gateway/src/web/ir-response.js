(() => {
  const factory = __makeIR,
    model = __protocolModel,
    states = new WeakMap();
  const maxFrame = 1 << 20;
  const clean = (o) =>
    Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined));
  function usage(v, d) {
    if (!v) return undefined;
    if (d === "anthropic.messages")
      return clean({
        input: v.input_tokens,
        output: v.output_tokens,
        cached: v.cache_read_input_tokens,
      });
    if (d === "gemini.generate")
      return clean({
        input: v.promptTokenCount,
        output: v.candidatesTokenCount,
        cached: v.cachedContentTokenCount,
        reasoning: v.thoughtsTokenCount,
        total: v.totalTokenCount,
      });
    return clean({
      input: v.input_tokens ?? v.prompt_tokens,
      output: v.output_tokens ?? v.completion_tokens,
      cached:
        v.input_tokens_details?.cached_tokens ??
        v.prompt_tokens_details?.cached_tokens,
      reasoning:
        v.output_tokens_details?.reasoning_tokens ??
        v.completion_tokens_details?.reasoning_tokens,
      total: v.total_tokens,
    });
  }
  function emitUsage(u, d) {
    if (!u) return undefined;
    if (d === "anthropic.messages")
      return clean({
        input_tokens: u.input,
        output_tokens: u.output,
        cache_read_input_tokens: u.cached,
      });
    if (d === "gemini.generate")
      return clean({
        promptTokenCount: u.input,
        candidatesTokenCount: u.output,
        cachedContentTokenCount: u.cached,
        thoughtsTokenCount: u.reasoning,
        totalTokenCount: u.total,
      });
    if (d === "openai.chat")
      return clean({
        prompt_tokens: u.input,
        completion_tokens: u.output,
        total_tokens: u.total,
        prompt_tokens_details:
          u.cached === undefined ? undefined : { cached_tokens: u.cached },
        completion_tokens_details:
          u.reasoning === undefined
            ? undefined
            : { reasoning_tokens: u.reasoning },
      });
    return clean({
      input_tokens: u.input,
      output_tokens: u.output,
      total_tokens: u.total,
      input_tokens_details:
        u.cached === undefined ? undefined : { cached_tokens: u.cached },
      output_tokens_details:
        u.reasoning === undefined
          ? undefined
          : { reasoning_tokens: u.reasoning },
    });
  }
  const finish = (v) =>
    ["tool_use", "tool_calls"].includes(v)
      ? "tool"
      : ["max_tokens", "length", "MAX_TOKENS", "incomplete"].includes(v)
        ? "length"
        : "stop";
  const endReason = (v, d) =>
    d === "anthropic.messages"
      ? { tool: "tool_use", length: "max_tokens", stop: "end_turn" }[v]
      : d === "gemini.generate"
        ? v === "length"
          ? "MAX_TOKENS"
          : "STOP"
        : v === "tool"
          ? "tool_calls"
          : v === "length"
            ? "length"
            : "stop";
  function normalize(value, d) {
    if (value.error) throw new Error(value.error.message ?? "upstream error");
    if (d === "anthropic.messages")
      return {
        id: value.id,
        model: value.model,
        content: (value.content ?? []).map((b) => model.block(b, d)),
        usage: usage(value.usage, d),
        finish: finish(value.stop_reason),
      };
    if (d === "openai.responses")
      return {
        id: value.id,
        model: value.model,
        content: (value.output ?? []).flatMap((m) =>
          m.type === "message"
            ? (m.content ?? []).map((b) => model.block(b, d))
            : [model.block(m, d)],
        ),
        usage: usage(value.usage, d),
        finish:
          value.status === "incomplete"
            ? "length"
            : (value.output ?? []).some((m) => m.type === "function_call")
              ? "tool"
              : "stop",
      };
    if (d === "openai.chat") {
      if (value.choices?.length !== 1)
        throw new TypeError("exactly one response choice is supported");
      const c = value.choices[0];
      return {
        id: value.id,
        model: value.model,
        content: model.decodeMessage(c.message, d).content,
        usage: usage(value.usage, d),
        finish: finish(c.finish_reason),
      };
    }
    if (value.candidates?.length !== 1)
      throw new TypeError("exactly one Gemini candidate is supported");
    const c = value.candidates[0];
    return {
      id: value.responseId,
      model: value.modelVersion,
      content: model.decodeMessage(c.content, d).content,
      usage: usage(value.usageMetadata, d),
      finish: finish(c.finishReason),
    };
  }
  function response(value, d) {
    for (const b of value.content) {
      if (b.signature || b.encryptedContent)
        throw new TypeError("opaque thinking cannot cross dialects");
      if (b.type === "thinking" && d === "openai.chat")
        throw new TypeError("Chat cannot express thinking blocks");
    }
    const u = emitUsage(value.usage, d);
    if (d === "anthropic.messages")
      return clean({
        id: value.id,
        type: "message",
        role: "assistant",
        model: value.model,
        content: value.content.map((b) => model.emitBlock(b, d)),
        stop_reason: endReason(value.finish, d),
        stop_sequence: null,
        usage: u,
      });
    if (d === "openai.responses") {
      const output = [];
      for (const b of value.content) {
        if (["tool_use", "thinking"].includes(b.type))
          output.push(model.emitBlock(b, d));
        else
          output.push({
            type: "message",
            role: "assistant",
            status: "completed",
            content: [{ type: "output_text", text: b.text, annotations: [] }],
          });
      }
      return clean({
        id: value.id,
        object: "response",
        model: value.model,
        status: value.finish === "length" ? "incomplete" : "completed",
        output,
        usage: u,
      });
    }
    if (d === "openai.chat") {
      const m = model.encodeMessage(
        { role: "assistant", content: value.content },
        d,
      );
      m.content =
        value.content
          .filter((b) => b.type === "text")
          .map((b) => b.text)
          .join("") || null;
      return clean({
        id: value.id,
        object: "chat.completion",
        model: value.model,
        choices: [
          { index: 0, message: m, finish_reason: endReason(value.finish, d) },
        ],
        usage: u,
      });
    }
    return clean({
      responseId: value.id,
      modelVersion: value.model,
      candidates: [
        {
          index: 0,
          content: model.encodeMessage(
            { role: "assistant", content: value.content },
            d,
          ),
          finishReason: endReason(value.finish, d),
        },
      ],
      usageMetadata: u,
    });
  }
  class Decoder {
    constructor(d) {
      this.d = d;
      this.started = false;
      this.blocks = new Map();
      this.ended = false;
      this.pendingFinish = "stop";
      this.geminiIndex = 0;
      this.geminiPart = new Map();
    }
    start(v, out) {
      if (!this.started) {
        this.started = true;
        out.push({
          type: "start",
          id: v.id ?? v.responseId,
          model: v.model ?? v.modelVersion,
        });
      }
    }
    block(index, b, out) {
      if (!this.blocks.has(index)) {
        this.blocks.set(index, b);
        out.push({ type: "block", index, block: b });
      }
    }
    close(out, reason) {
      if (this.ended) return;
      this.ended = true;
      for (const index of this.blocks.keys())
        out.push({ type: "block_end", index });
      out.push({ type: "end", finish: reason ?? this.pendingFinish });
    }
    frame(event, data) {
      const out = [];
      if (data === "[DONE]") {
        this.close(out);
        return out;
      }
      if (!data || this.ended) return out;
      const v = JSON.parse(data);
      if (v.error || event === "error" || v.type === "error") {
        this.ended = true;
        return [
          {
            type: "error",
            message: v.error?.message ?? v.message ?? "upstream error",
          },
        ];
      }
      const d = this.d;
      if (d === "anthropic.messages") {
        const t = v.type ?? event;
        if (t === "message_start") {
          this.start(v.message, out);
          if (v.message?.usage)
            out.push({ type: "usage", value: usage(v.message.usage, d) });
        } else if (t === "content_block_start") {
          this.start({}, out);
          const b = model.block(v.content_block, d);
          this.block(v.index, b, out);
          if (b.text)
            out.push({
              type: b.type === "thinking" ? "thinking" : "text",
              index: v.index,
              text: b.text,
            });
        } else if (t === "content_block_delta") {
          const delta = v.delta;
          if (delta.type === "text_delta")
            out.push({ type: "text", index: v.index, text: delta.text });
          else if (delta.type === "thinking_delta")
            out.push({
              type: "thinking",
              index: v.index,
              text: delta.thinking,
            });
          else if (delta.type === "input_json_delta")
            out.push({
              type: "arguments",
              index: v.index,
              text: delta.partial_json,
            });
          else if (delta.type === "signature_delta")
            throw new TypeError("signed thinking cannot cross dialects");
          else throw new TypeError("unsupported Anthropic delta");
        } else if (t === "content_block_stop") {
          out.push({ type: "block_end", index: v.index });
          this.blocks.delete(v.index);
        } else if (t === "message_delta") {
          this.pendingFinish = finish(v.delta?.stop_reason);
          if (v.usage) out.push({ type: "usage", value: usage(v.usage, d) });
        } else if (t === "message_stop") this.close(out);
        else if (t !== "ping")
          throw new TypeError(`unsupported Anthropic event: ${t}`);
      } else if (d === "openai.responses") {
        const t = v.type ?? event;
        const part = `${v.output_index}:${t.includes("reasoning") ? "thinking" : "text"}:${v.content_index ?? v.summary_index ?? 0}`;
        const tool = `${v.output_index}:tool`;
        if (t === "response.created" || t === "response.in_progress")
          this.start(v.response ?? {}, out);
        else if (t === "response.output_item.added") {
          this.start({}, out);
          if (v.item.type === "function_call")
            this.block(
              tool,
              {
                type: "tool_use",
                id: v.item.call_id,
                name: v.item.name,
                input: {},
              },
              out,
            );
          else if (!["message", "reasoning"].includes(v.item.type))
            throw new TypeError(`unsupported Responses item: ${v.item.type}`);
        } else if (t === "response.content_part.added") {
          this.start({}, out);
          if (v.part && v.part.type !== "output_text")
            throw new TypeError("unsupported response content part");
          this.block(part, { type: "text", text: "" }, out);
        } else if (
          t === "response.output_text.delta" ||
          t === "response.reasoning_summary_text.delta"
        ) {
          this.start({}, out);
          const thinking = t.includes("reasoning");
          this.block(
            part,
            { type: thinking ? "thinking" : "text", text: "" },
            out,
          );
          out.push({
            type: thinking ? "thinking" : "text",
            index: part,
            text: v.delta,
          });
        } else if (t === "response.function_call_arguments.delta")
          out.push({ type: "arguments", index: tool, text: v.delta });
        else if (t === "response.output_item.done") {
          if (v.item?.encrypted_content)
            throw new TypeError("encrypted thinking cannot cross dialects");
          for (const index of this.blocks.keys()) {
            if (String(index).startsWith(`${v.output_index}:`)) {
              out.push({ type: "block_end", index });
              this.blocks.delete(index);
            }
          }
        } else if (t === "response.completed" || t === "response.incomplete") {
          if (v.response?.usage)
            out.push({ type: "usage", value: usage(v.response.usage, d) });
          this.close(
            out,
            t === "response.incomplete"
              ? "length"
              : v.response?.output?.some((x) => x.type === "function_call")
                ? "tool"
                : "stop",
          );
        } else if (t === "response.failed")
          out.push({
            type: "error",
            message: v.response?.error?.message ?? "response failed",
          });
        else if (
          ![
            "response.output_text.done",
            "response.content_part.done",
            "response.function_call_arguments.done",
            "response.reasoning_summary_part.added",
            "response.reasoning_summary_part.done",
            "response.reasoning_summary_text.done",
            "response.queued",
          ].includes(t)
        )
          throw new TypeError(`unsupported Responses event: ${t}`);
      } else if (d === "openai.chat") {
        this.start(v, out);
        if (v.usage) out.push({ type: "usage", value: usage(v.usage, d) });
        if (v.choices?.length > 1)
          throw new TypeError("multiple choices unsupported");
        for (const c of v.choices ?? []) {
          const delta = c.delta ?? {};
          if (delta.content) {
            this.block(0, { type: "text", text: "" }, out);
            out.push({ type: "text", index: 0, text: delta.content });
          }
          if (delta.reasoning_content) {
            this.block(100000, { type: "thinking", text: "" }, out);
            out.push({
              type: "thinking",
              index: 100000,
              text: delta.reasoning_content,
            });
          }
          for (const t of delta.tool_calls ?? []) {
            const index = 1 + t.index;
            this.block(
              index,
              { type: "tool_use", id: t.id, name: t.function?.name, input: {} },
              out,
            );
            if (t.function?.arguments)
              out.push({
                type: "arguments",
                index,
                text: t.function.arguments,
              });
          }
          if (c.finish_reason) this.pendingFinish = finish(c.finish_reason);
        }
      } else {
        this.start(v, out);
        if (v.usageMetadata)
          out.push({ type: "usage", value: usage(v.usageMetadata, d) });
        if (v.candidates?.length > 1)
          throw new TypeError("multiple candidates unsupported");
        for (const c of v.candidates ?? []) {
          for (const [i, p] of (c.content?.parts ?? []).entries()) {
            if (p.thoughtSignature)
              throw new TypeError("signed thought cannot cross dialects");
            const b = model.block(p, d);
            const previous = this.geminiPart.get(i);
            const index =
              b.type !== "tool_use" && previous?.type === b.type
                ? previous.index
                : this.geminiIndex++;
            this.geminiPart.set(i, { type: b.type, index });
            this.block(index, { ...b, text: "" }, out);
            if (b.type === "tool_use")
              out.push({
                type: "arguments",
                index,
                text: JSON.stringify(b.input),
              });
            else
              out.push({
                type: b.type === "thinking" ? "thinking" : "text",
                index,
                text: b.text ?? "",
              });
          }
          if (c.finishReason) this.close(out, finish(c.finishReason));
        }
      }
      return out;
    }
  }
  class Emitter {
    constructor(d) {
      this.d = d;
      this.id = undefined;
      this.model = undefined;
      this.blocks = new Map();
      this.usage = undefined;
      this.started = false;
      this.seq = 0;
    }
    frame(event, data) {
      return `${event ? `event: ${event}\n` : ""}data: ${JSON.stringify(clean(data))}\n\n`;
    }
    send(e) {
      const d = this.d;
      let out = "";
      const emit = (event, data) => {
        out += this.frame(event, data);
      };
      const chat = (data) =>
        emit("", {
          id: this.id,
          object: "chat.completion.chunk",
          model: this.model,
          ...data,
        });
      if (e.type === "start") {
        if (this.started) return "";
        this.started = true;
        this.id = e.id;
        this.model = e.model;
        if (d === "anthropic.messages")
          emit("message_start", {
            type: "message_start",
            message: clean({
              id: this.id,
              type: "message",
              role: "assistant",
              model: this.model,
              content: [],
              stop_reason: null,
              usage: {},
            }),
          });
        else if (d === "openai.responses")
          emit("response.created", {
            type: "response.created",
            sequence_number: this.seq++,
            response: clean({
              id: this.id,
              object: "response",
              model: this.model,
              status: "in_progress",
              output: [],
            }),
          });
        else if (d === "openai.chat")
          chat({
            choices: [
              {
                index: 0,
                delta: { role: "assistant", content: "" },
                finish_reason: null,
              },
            ],
          });
        return out;
      }
      if (e.type === "error") {
        return this.frame(
          d === "anthropic.messages"
            ? "error"
            : d === "openai.responses"
              ? "error"
              : "",
          {
            type: "error",
            error: { type: "upstream_error", message: e.message },
          },
        );
      }
      if (e.type === "usage") {
        this.usage = { ...this.usage, ...e.value };
        return "";
      }
      if (e.type === "block") {
        if (e.block.signature || e.block.encryptedContent)
          throw new TypeError("opaque thinking cannot cross dialects");
        if (d === "openai.chat" && e.block.type === "thinking")
          throw new TypeError("Chat cannot express thinking blocks");
        const b = {
          ...e.block,
          index: this.blocks.size,
          text: "",
          arguments: "",
          closed: false,
        };
        this.blocks.set(e.index, b);
        if (d === "anthropic.messages")
          emit("content_block_start", {
            type: "content_block_start",
            index: b.index,
            content_block: model.emitBlock({ ...b, input: {} }, d),
          });
        else if (d === "openai.responses") {
          const item =
            b.type === "tool_use"
              ? {
                  type: "function_call",
                  id: b.id,
                  call_id: b.id,
                  name: b.name,
                  arguments: "",
                  status: "in_progress",
                }
              : b.type === "thinking"
                ? { type: "reasoning", summary: [] }
                : {
                    type: "message",
                    role: "assistant",
                    content: [],
                    status: "in_progress",
                  };
          emit("response.output_item.added", {
            type: "response.output_item.added",
            sequence_number: this.seq++,
            output_index: b.index,
            item,
          });
          if (b.type === "text")
            emit("response.content_part.added", {
              type: "response.content_part.added",
              sequence_number: this.seq++,
              output_index: b.index,
              content_index: 0,
              part: { type: "output_text", text: "", annotations: [] },
            });
        } else if (d === "openai.chat" && b.type === "tool_use")
          chat({
            choices: [
              {
                index: 0,
                delta: {
                  tool_calls: [
                    {
                      index: b.index,
                      id: b.id,
                      type: "function",
                      function: { name: b.name, arguments: "" },
                    },
                  ],
                },
                finish_reason: null,
              },
            ],
          });
        return out;
      }
      const b = this.blocks.get(e.index);
      if (["text", "thinking", "arguments"].includes(e.type)) {
        if (!b) throw new TypeError("delta has no content start");
        const field = e.type === "arguments" ? "arguments" : "text";
        if (
          d === "openai.responses" ||
          (d === "gemini.generate" && field === "arguments")
        )
          b[field] += e.text;
        if (new TextEncoder().encode(b[field]).length > maxFrame)
          throw new RangeError("converted content block exceeds 1 MiB");
        if (d === "anthropic.messages")
          emit("content_block_delta", {
            type: "content_block_delta",
            index: b.index,
            delta:
              e.type === "arguments"
                ? { type: "input_json_delta", partial_json: e.text }
                : e.type === "thinking"
                  ? { type: "thinking_delta", thinking: e.text }
                  : { type: "text_delta", text: e.text },
          });
        else if (d === "openai.responses") {
          const type =
            e.type === "arguments"
              ? "response.function_call_arguments.delta"
              : e.type === "thinking"
                ? "response.reasoning_summary_text.delta"
                : "response.output_text.delta";
          emit(type, {
            type,
            sequence_number: this.seq++,
            output_index: b.index,
            content_index: 0,
            delta: e.text,
          });
        } else if (d === "openai.chat")
          chat({
            choices: [
              {
                index: 0,
                delta:
                  e.type === "arguments"
                    ? {
                        tool_calls: [
                          { index: b.index, function: { arguments: e.text } },
                        ],
                      }
                    : { content: e.text },
                finish_reason: null,
              },
            ],
          });
        else if (e.type !== "arguments")
          emit("", {
            responseId: this.id,
            modelVersion: this.model,
            candidates: [
              {
                index: 0,
                content: {
                  role: "model",
                  parts: [
                    e.type === "thinking"
                      ? { text: e.text, thought: true }
                      : { text: e.text },
                  ],
                },
              },
            ],
          });
        return out;
      }
      if (e.type === "block_end") {
        if (!b || b.closed) return "";
        b.closed = true;
        if (d === "anthropic.messages")
          emit("content_block_stop", {
            type: "content_block_stop",
            index: b.index,
          });
        else if (d === "openai.responses") {
          let item;
          if (b.type === "tool_use")
            item = {
              type: "function_call",
              call_id: b.id,
              name: b.name,
              arguments: b.arguments,
              status: "completed",
            };
          else if (b.type === "thinking")
            item = {
              type: "reasoning",
              summary: [{ type: "summary_text", text: b.text }],
            };
          else
            item = {
              type: "message",
              role: "assistant",
              status: "completed",
              content: [{ type: "output_text", text: b.text, annotations: [] }],
            };
          emit("response.output_item.done", {
            type: "response.output_item.done",
            sequence_number: this.seq++,
            output_index: b.index,
            item,
          });
        } else if (d === "gemini.generate" && b.type === "tool_use")
          emit("", {
            responseId: this.id,
            candidates: [
              {
                index: 0,
                content: {
                  role: "model",
                  parts: [
                    {
                      functionCall: {
                        id: b.id,
                        name: b.name,
                        args: JSON.parse(b.arguments || "{}"),
                      },
                    },
                  ],
                },
              },
            ],
          });
        return out;
      }
      if (e.type === "end") {
        for (const [index, block] of this.blocks)
          if (!block.closed) out += this.send({ type: "block_end", index });
        const u = emitUsage(this.usage, d);
        if (d === "anthropic.messages") {
          emit("message_delta", {
            type: "message_delta",
            delta: { stop_reason: endReason(e.finish, d), stop_sequence: null },
            usage: u,
          });
          emit("message_stop", { type: "message_stop" });
        } else if (d === "openai.responses") {
          const type =
            e.finish === "length"
              ? "response.incomplete"
              : "response.completed";
          emit(type, {
            type,
            sequence_number: this.seq++,
            response: response(
              {
                id: this.id,
                model: this.model,
                content: [...this.blocks.values()].map((b) =>
                  b.type === "tool_use"
                    ? { ...b, input: JSON.parse(b.arguments || "{}") }
                    : b,
                ),
                usage: this.usage,
                finish: e.finish,
              },
              d,
            ),
          });
        } else if (d === "openai.chat") {
          chat({
            choices: [
              { index: 0, delta: {}, finish_reason: endReason(e.finish, d) },
            ],
          });
          if (u) chat({ choices: [], usage: u });
          out += "data: [DONE]\n\n";
        } else
          emit("", {
            responseId: this.id,
            modelVersion: this.model,
            candidates: [{ index: 0, finishReason: endReason(e.finish, d) }],
            usageMetadata: u,
          });
      }
      return out;
    }
  }
  function convertSSE(body, source, target) {
    const decoder = new Decoder(source),
      emitter = new Emitter(target);
    let buffer = "",
      failed = false;
    const process = (frame, controller) => {
      let event = "",
        lines = [];
      for (const line of frame.split("\n")) {
        if (line.startsWith(":")) continue;
        const colon = line.indexOf(":");
        const key = colon < 0 ? line : line.slice(0, colon);
        let value = colon < 0 ? "" : line.slice(colon + 1);
        if (value.startsWith(" ")) value = value.slice(1);
        if (key === "event") event = value;
        if (key === "data") lines.push(value);
      }
      if (!lines.length) return;
      for (const e of decoder.frame(event, lines.join("\n"))) {
        const out = emitter.send(e);
        if (out) controller.enqueue(out);
        if (e.type === "error") {
          failed = true;
          controller.terminate();
          break;
        }
      }
    };
    return body
      .pipeThrough(new TextDecoderStream())
      .pipeThrough(
        new TransformStream({
          transform(chunk, c) {
            if (failed) return;
            buffer += chunk;
            try {
              for (;;) {
                const match = /\r?\n\r?\n/.exec(buffer);
                if (!match) break;
                const frame = buffer
                  .slice(0, match.index)
                  .replace(/\r\n/g, "\n");
                buffer = buffer.slice(match.index + match[0].length);
                if (new TextEncoder().encode(frame).length > maxFrame)
                  throw new RangeError("SSE frame exceeds 1 MiB");
                process(frame, c);
                if (failed) return;
              }
              if (new TextEncoder().encode(buffer).length > maxFrame)
                throw new RangeError("SSE frame exceeds 1 MiB");
            } catch (e) {
              failed = true;
              c.enqueue(emitter.send({ type: "error", message: e.message }));
              c.terminate();
            }
          },
          flush(c) {
            if (failed) return;
            try {
              if (buffer.trim()) process(buffer.replace(/\r\n/g, "\n"), c);
              if (!decoder.ended && !failed)
                throw new Error("upstream stream ended before terminal event");
            } catch (e) {
              c.enqueue(emitter.send({ type: "error", message: e.message }));
            }
          },
        }),
      )
      .pipeThrough(new TextEncoderStream());
  }
  globalThis.__makeIR = (bodyOf, streamOf) => {
    const api = factory(bodyOf, streamOf);
    return {
      from(input, d) {
        if (!(input instanceof Response)) return api.from(input, d);
        if (!model.dialects.includes(d))
          throw new TypeError("explicit response dialect required");
        if (input.bodyUsed || input.body?.locked)
          throw new TypeError("response body consumed");
        const content = input.headers.get("content-type")?.split(";")[0].trim();
        if (!["application/json", "text/event-stream"].includes(content))
          throw new TypeError("response Content-Type must be JSON or SSE");
        const ir = Object.freeze({ protocol: d });
        states.set(ir, { input, d, content, used: false });
        return ir;
      },
      to(ir, target) {
        const state = states.get(ir);
        if (!state) return api.to(ir, target);
        if (state.used) throw new TypeError("response IR is single-use");
        target ??= state.d;
        if (!model.dialects.includes(target))
          throw new TypeError("unsupported target dialect");
        state.used = true;
        if (target === state.d) return state.input.body;
        if (state.content === "text/event-stream")
          return convertSSE(state.input.body, state.d, target);
        return new ReadableStream({
          async start(controller) {
            try {
              const result = response(
                normalize(await state.input.json(), state.d),
                target,
              );
              controller.enqueue(
                new TextEncoder().encode(JSON.stringify(result)),
              );
              controller.close();
            } catch (e) {
              controller.error(e);
            }
          },
          cancel(reason) {
            return state.input.body?.cancel(reason);
          },
        });
      },
    };
  };
  Object.assign(__protocolModel, {
    normalizeResponse: normalize,
    emitResponse: response,
    Decoder,
    Emitter,
    convertSSE,
  });
})();
