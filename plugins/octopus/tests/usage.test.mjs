import { test } from 'node:test';
import assert from 'node:assert/strict';
import plugin from '../index.mjs';

async function invoke(path, text, type = 'text/event-stream', status = 200) {
  const bytes = new TextEncoder().encode(text);
  const reports = [];
  let sent;
  const original = globalThis.fetch;
  globalThis.fetch = async (_url, init) => {
    sent = JSON.parse(init.body);
    let offset = 0;
    return new Response(new ReadableStream({
      pull(controller) {
        if (offset === bytes.length) return controller.close();
        // Split CRLF, UTF-8 characters and event fields across chunks.
        controller.enqueue(bytes.slice(offset, offset += Math.min(7, bytes.length - offset)));
      },
    }), { status, headers: { 'content-type': type } });
  };
  try {
    const response = await plugin.fetch(new Request('http://local/octopus/v1/' + path, {
      method: 'POST', body: JSON.stringify({ stream: type === 'text/event-stream', messages: [] }),
    }), { API_KEY: 'fixture', IDENTITY: { userId: 'user-fixture', keyId: 'key-fixture' }, USAGE: { record: row => reports.push(row) } });
    assert.equal(await response.text(), text, 'response bytes must not change');
    assert.equal(response.status, status);
    return { reports, sent };
  } finally { globalThis.fetch = original; }
}

test('chat SSE: split UTF-8, multiline data, repeated cumulative usage, once-only reporting', async () => {
  const result = await invoke('chat/completions', ': ping\r\ndata: {"text":"你好"}\r\n\r\ndata: {"usage":\r\ndata: {"prompt_tokens":2,"completion_tokens":4}}\r\n\r\ndata: {"usage":{"prompt_tokens":2,"completion_tokens":4}}\r\n\r\ndata: [DONE]\r\n\r\n');
  assert.equal(result.sent.stream_options.include_usage, true);
  assert.equal(result.reports.length, 1);
  assert.equal(result.reports[0].total_tokens, 6);
  assert.equal(result.reports[0].user_id, 'user-fixture');
  assert.equal(result.reports[0].complete, true);
});

test('Responses terminal usage includes cache and reasoning details', async () => {
  const event = { type: 'response.completed', response: { model: 'actual-model', usage: { input_tokens: 8, output_tokens: 6, total_tokens: 14, input_tokens_details: { cached_tokens: 3 }, output_tokens_details: { reasoning_tokens: 4 } } } };
  const { reports } = await invoke('responses', `event: response.completed\ndata: ${JSON.stringify(event)}\n\n`);
  assert.equal(reports.length, 1);
  assert.equal(reports[0].cache_read_tokens, 3);
  assert.equal(reports[0].reasoning_tokens, 4);
  assert.equal(reports[0].model, 'actual-model');
  assert.equal(reports[0].total_tokens, 14);
});

test('Anthropic start + cumulative deltas merge, including cache input; zero is valid', async () => {
  const frames = [
    { type: 'message_start', message: { usage: { input_tokens: 0, cache_read_input_tokens: 3, cache_creation_input_tokens: 2, output_tokens: 0 } } },
    { type: 'message_delta', usage: { output_tokens: 1 } },
    { type: 'message_delta', usage: { output_tokens: 4 } },
    { type: 'message_stop' },
  ];
  const { reports } = await invoke('messages', frames.map(e => `data: ${JSON.stringify(e)}\r\r`).join(''));
  assert.equal(reports.length, 1);
  assert.equal(reports[0].input_tokens, 5);
  assert.equal(reports[0].output_tokens, 4);
  assert.equal(reports[0].total_tokens, 9);
});

test('JSON usage recorded and HTTP errors/missing usage are not invented', async () => {
  const { reports } = await invoke('chat/completions', '{"usage":{"prompt_tokens":0,"completion_tokens":0}}', 'application/json');
  assert.equal(reports.length, 1);
  assert.equal(reports[0].total_tokens, 0);
  assert.equal((await invoke('messages', '{"error":"limited"}', 'application/json', 429)).reports.length, 0);
  assert.equal((await invoke('messages', '{"text":"no usage"}', 'application/json')).reports.length, 0);
});

test('EOF without terminal event reports known usage as incomplete; invalid numbers ignored', async () => {
  const { reports } = await invoke('responses', 'data: {"usage":{"input_tokens":2,"output_tokens":-1}}\n\n');
  assert.equal(reports.length, 1);
  assert.equal(reports[0].input_tokens, 2);
  assert.equal(reports[0].complete, false);
  assert.equal(reports[0].output_tokens, undefined);
  assert.equal(reports[0].total_tokens, undefined);
});
