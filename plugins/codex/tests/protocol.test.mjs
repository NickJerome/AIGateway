import test from "node:test";
import assert from "node:assert/strict";
import { clientKey, prepare, upstreamHeaders } from "../src/codex.js";
import { aggregate, stream } from "../src/stream.js";
const key = { id: "alice", key: "client-secret", models: ["alias"] };
const env = {
  CLIENT_API_KEYS: [key],
  ADMIN_TOKEN: "admin-secret",
  MODEL_MAP: { alias: "backend-model" },
};
const frame = (v) =>
  "data: " + (typeof v === "string" ? v : JSON.stringify(v)) + "\r\n\r\n";
function source(events, step = 7) {
  const data = new TextEncoder().encode(events.map(frame).join(""));
  let at = 0;
  return new ReadableStream({
    pull(c) {
      if (at === data.length) {
        c.close();
        return;
      }
      c.enqueue(data.slice(at, at + step));
      at = Math.min(data.length, at + step);
    },
  });
}
const complete = {
  type: "response.completed",
  response: { id: "resp-test", status: "completed", output: [] },
};
test("client authentication fails closed and separates administrator credentials", () => {
  for (const token of ["", "admin-secret", "wrong"])
    assert.throws(
      () =>
        clientKey(
          new Request("http://x", {
            headers: { authorization: "Bearer " + token },
          }),
          env,
        ),
      (e) => e.status === 401,
    );
  assert.equal(
    clientKey(
      new Request("http://x", {
        headers: { authorization: "Bearer client-secret" },
      }),
      env,
    ).id,
    "alice",
  );
  assert.throws(
    () => clientKey(new Request("http://x"), {}),
    (e) => e.status === 401,
  );
  assert.throws(() =>
    clientKey(
      new Request("http://x", {
        headers: { authorization: "Bearer client-secret" },
      }),
      { ...env, CLIENT_API_KEYS: [{ ...key, enabled: false }] },
    ),
  );
});
test("Codex request changes transport fields and preserves opaque native items", () => {
  const raw = {
    model: "alias",
    stream: false,
    input: [
      { role: "system", content: "rules" },
      { type: "reasoning", encrypted_content: "opaque" },
      { type: "function_call_output", call_id: "call-1", output: "done" },
    ],
    tools: [{ type: "web_search_preview" }],
    parallel_tool_calls: false,
    max_output_tokens: 40,
    temperature: 0.2,
  };
  const out = prepare(raw, env, key);
  assert.equal(out.stream, true);
  assert.equal(out.store, false);
  assert.equal(out.model, "backend-model");
  assert.equal(out.input[0].role, "developer");
  assert.equal(out.input[1].encrypted_content, "opaque");
  assert.equal(out.input[2].call_id, "call-1");
  assert.equal(out.tools[0].type, "web_search");
  assert.equal(out.parallel_tool_calls, false);
  assert.equal(out.max_output_tokens, undefined);
  assert.equal(out.temperature, undefined);
  assert.equal(raw.input[0].role, "system");
  assert.deepEqual(out.include, ["reasoning.encrypted_content"]);
});
test("native-only constraints and model policy reject before upstream calls", () => {
  for (const extra of [
    { previous_response_id: "resp-x" },
    { conversation: "conv-x" },
    { background: true },
    { stream: "false" },
  ])
    assert.throws(() =>
      prepare({ model: "alias", input: "hi", ...extra }, env, key),
    );
  assert.throws(
    () => prepare({ model: "other", input: "hi" }, env, key),
    (e) => e.status === 403,
  );
  assert.equal(
    prepare({ model: "alias", input: "hi" }, env, key, true).stream,
    undefined,
  );
  assert.throws(() =>
    prepare({ model: "alias", input: "hi", stream: true }, env, key, true),
  );
});
test("only protocol headers pass to upstream; caller credentials cannot override OAuth", () => {
  const req = new Request("http://x", {
    headers: {
      authorization: "Bearer client-secret",
      cookie: "cookie-secret",
      "chatgpt-account-id": "spoof",
      "x-codex-turn-state": "opaque-turn",
    },
  });
  const headers = upstreamHeaders(
    req,
    { access: "upstream-secret", accountId: "real-account" },
    env,
    false,
  );
  assert.equal(headers.get("authorization"), "Bearer upstream-secret");
  assert.equal(headers.get("chatgpt-account-id"), "real-account");
  assert.equal(headers.get("cookie"), null);
  assert.equal(headers.get("x-codex-turn-state"), "opaque-turn");
});
test("non-stream aggregation reconstructs final output and retains encrypted reasoning", async () => {
  const events = [
    {
      type: "response.output_item.done",
      output_index: 1,
      item: {
        type: "message",
        content: [{ type: "output_text", text: "你好" }],
      },
    },
    {
      type: "response.output_item.done",
      output_index: 0,
      item: { type: "reasoning", encrypted_content: "opaque" },
    },
    complete,
  ];
  for (const size of [1, 7, 65536]) {
    const value = await aggregate(source(events, size), async () => {});
    assert.equal(value.output[0].encrypted_content, "opaque");
    assert.equal(value.output[1].content[0].text, "你好");
  }
});
test("bare DONE and failed terminal never become successful JSON responses", async () => {
  await assert.rejects(
    aggregate(source(["[DONE]"]), async () => {}),
    /before.*terminal/,
  );
  await assert.rejects(
    aggregate(
      source([
        {
          type: "response.completed",
          response: { status: "failed", error: { message: "denied" } },
        },
      ]),
      async () => {},
    ),
    /denied/,
  );
});
test("SSE remains incremental and reports missing terminal", async () => {
  const text = await new Response(
    stream(
      source([
        { type: "response.output_text.delta", delta: "partial" },
        "[DONE]",
      ]),
      async () => {},
    ),
  ).text();
  assert.ok(text.includes("partial"));
  assert.ok(text.includes("event: error"));
  assert.ok(text.includes("before a Responses terminal"));
  const completeText = await new Response(
    stream(source([complete]), async () => {}),
  ).text();
  assert.ok(completeText.includes("response.completed"));
  assert.ok(!completeText.includes("event: error"));
});
test("cancelling output cancels a pending source read", async () => {
  let cancelled = false;
  const body = new ReadableStream({
    cancel() {
      cancelled = true;
    },
  });
  const reader = stream(body, async () => {}).getReader();
  const pending = reader.read();
  await new Promise((resolve) => setTimeout(resolve, 10));
  await reader.cancel();
  await pending;
  assert.equal(cancelled, true);
});
