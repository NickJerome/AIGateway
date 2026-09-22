// Copyright (c) Cloudflare, Inc. Apache-2.0; see workerd.LICENSE.
// Port from workerd 1ed4b65ccdf861023a82f9639fd2d6d8d01cd67e.
// Only URL is adapted to a local refused connection; assertions unchanged.
function doesNotThrow(fn) { fn(); }


// websocket-constructor-test.js:16
const emptyProtocolsArray = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/', []);
    strictEqual(ws.url, 'ws://127.0.0.1:9/');
    strictEqual(ws.protocol, '');
    strictEqual(ws.readyState, WebSocket.CONNECTING);
    ws.close();
  },
};

// websocket-constructor-test.js:27
const singleProtocolString = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/', 'chat');
    strictEqual(ws.url, 'ws://127.0.0.1:9/');
    ws.close();
  },
};

// websocket-constructor-test.js:36
const invalidProtocolToken = {
  async test() {
    throws(
      () => new WebSocket('ws://127.0.0.1:9/', 'invalid protocol with spaces'),
      {
        name: 'SyntaxError',
      }
    );
  },
};

// websocket-constructor-test.js:48
const duplicateProtocols = {
  async test() {
    throws(() => new WebSocket('ws://127.0.0.1:9/', ['chat', 'chat']), {
      name: 'SyntaxError',
    });
  },
};

// websocket-constructor-test.js:59
const closeReasonTooLong = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/');
    // 124 ASCII bytes => 124 UTF-8 bytes, which exceeds the 123-byte limit.
    const longReason = 'a'.repeat(124);
    throws(() => ws.close(1000, longReason), {
      name: 'SyntaxError',
    });
    ws.close();
  },
};

// websocket-constructor-test.js:72
const closeReasonExact123Bytes = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/');
    const reason123 = 'a'.repeat(123);
    doesNotThrow(() => ws.close(1000, reason123));
  },
};

// websocket-constructor-test.js:82
const closeReasonMultibyteExceeds = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/');
    const multibyteReason = '\u00e9'.repeat(62); // 62 chars × 2 bytes = 124 bytes
    throws(() => ws.close(1000, multibyteReason), {
      name: 'SyntaxError',
    });
    ws.close();
  },
};

// websocket-constructor-test.js:96
const closeReasonUSVString = {
  async test() {
    // CloseEvent.reason is typed as USVString per the HTML spec,
    // so lone surrogates must be replaced with U+FFFD.
    const evt = new CloseEvent('close', { code: 1000, reason: '\uD807' });
    strictEqual(
      evt.reason,
      '\uFFFD',
      'CloseEvent reason should replace lone surrogate with U+FFFD'
    );

    // Multiple surrogates in a string.
    const evt2 = new CloseEvent('close', {
      code: 1000,
      reason: 'hello\uD800world\uDC00end',
    });
    strictEqual(
      evt2.reason,
      'hello\uFFFDworld\uFFFDend',
      'CloseEvent reason should replace each lone surrogate with U+FFFD'
    );

    // Properly paired surrogates should pass through unchanged.
    const evt3 = new CloseEvent('close', {
      code: 1000,
      reason: '\uD83D\uDE00',
    });
    strictEqual(
      evt3.reason,
      '\uD83D\uDE00',
      'Properly paired surrogates should not be replaced'
    );
  },
};

// websocket-constructor-test.js:132
const closeCode1000Succeeds = {
  async test() {
    const ws = new WebSocket('ws://127.0.0.1:9/');
    doesNotThrow(() => ws.close(1000));
  },
};

// websocket-constructor-test.js:140
const closeCode3000To4999Succeeds = {
  async test() {
    for (const code of [3000, 3500, 4000, 4999]) {
      const ws = new WebSocket('ws://127.0.0.1:9/');
      doesNotThrow(() => ws.close(code));
    }
  },
};

// websocket-constructor-test.js:151
const closeCodeSpecInvalid = {
  async test() {
    for (const code of [999, 1001, 1004, 1005, 1006, 1015, 2999, 5000]) {
      const ws = new WebSocket('ws://127.0.0.1:9/');
      throws(
        () => ws.close(code),
        { name: 'InvalidAccessError' },
        `close(${code}) should throw InvalidAccessError`
      );
      ws.close();
    }
  },
};

// websocket-constructor-test.js:246
const binaryMessageDeliveredAsArrayBuffer = {
  async test() {
    const pair = new WebSocketPair();
    const [client, server] = pair;
    server.accept();
    server.binaryType = 'arraybuffer';

    const data = new Uint8Array([10, 20, 30]);
    const received = new Promise((resolve) => {
      server.addEventListener('message', (event) => resolve(event.data));
    });

    client.accept();
    client.send(data);

    const msg = await received;
    strictEqual(msg instanceof ArrayBuffer, true);
    deepStrictEqual(new Uint8Array(msg), data);

    client.close();
    server.close();
  },
};

// events-test.js:753
const messageEventSpecInit = {
  test() {
    const defaults = new MessageEvent('message');
    strictEqual(defaults.data, null);
    strictEqual(defaults.origin, '');
    strictEqual(defaults.lastEventId, '');
    strictEqual(defaults.source, null);
    deepStrictEqual(defaults.ports, []);
    strictEqual(defaults.bubbles, false);
    strictEqual(defaults.cancelable, false);
    strictEqual(defaults.composed, false);

    const data = { hello: 'world' };
    const event = new MessageEvent('message', {
      data,
      origin: 'https://example.org',
      lastEventId: '42',
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    strictEqual(event.data, data);
    strictEqual(event.origin, 'https://example.org');
    strictEqual(event.lastEventId, '42');
    strictEqual(event.bubbles, true);
    strictEqual(event.cancelable, true);
    strictEqual(event.composed, true);
    event.preventDefault();
    strictEqual(event.defaultPrevented, true);
  },
};

// events-test.js:786
const closeEventSpecInit = {
  test() {
    const defaults = new CloseEvent('close');
    strictEqual(defaults.code, 0);
    strictEqual(defaults.reason, '');
    strictEqual(defaults.wasClean, false);
    strictEqual(defaults.bubbles, false);
    strictEqual(defaults.cancelable, false);
    strictEqual(defaults.composed, false);

    const event = new CloseEvent('close', {
      code: 1000,
      reason: 'done',
      wasClean: true,
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    strictEqual(event.code, 1000);
    strictEqual(event.reason, 'done');
    strictEqual(event.wasClean, true);
    strictEqual(event.bubbles, true);
    strictEqual(event.cancelable, true);
    strictEqual(event.composed, true);
  },
};

// events-test.js:814
const errorEventSpecInit = {
  test() {
    const defaults = new ErrorEvent('error');
    strictEqual(defaults.message, '');
    strictEqual(defaults.bubbles, false);
    strictEqual(defaults.cancelable, false);
    strictEqual(defaults.composed, false);

    const err = new Error('boom');
    const event = new ErrorEvent('error', {
      message: 'boom',
      filename: 'test.js',
      lineno: 1,
      colno: 2,
      error: err,
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    strictEqual(event.message, 'boom');
    strictEqual(event.filename, 'test.js');
    strictEqual(event.lineno, 1);
    strictEqual(event.colno, 2);
    strictEqual(event.error, err);
    strictEqual(event.bubbles, true);
    strictEqual(event.cancelable, true);
    strictEqual(event.composed, true);
    event.preventDefault();
    strictEqual(event.defaultPrevented, true);
  },
};

export default {async fetch(){await emptyProtocolsArray.test();await singleProtocolString.test();await invalidProtocolToken.test();await duplicateProtocols.test();await closeReasonTooLong.test();await closeReasonExact123Bytes.test();await closeReasonMultibyteExceeds.test();await closeReasonUSVString.test();await closeCode1000Succeeds.test();await closeCode3000To4999Succeeds.test();await closeCodeSpecInvalid.test();await binaryMessageDeliveredAsArrayBuffer.test();await messageEventSpecInit.test();await closeEventSpecInit.test();await errorEventSpecInit.test();return new Response('15 workerd groups passed');}}