// Copyright (c) 2023–2026 Cloudflare, Inc. Apache-2.0.
// Adapted test harness; assertions retain their upstream wording.
// Source commit: 3ea9117c64c9fd29d734a60e066ef2872ee479aa
const scheduler = {wait:ms=>new Promise(resolve=>setTimeout(resolve,ms))};
async function rejects(input, expected) {
 let error;try{await (typeof input==='function'?input():input);}catch(e){error=e;}
 ok(error!==undefined,'expected rejection');
 if(typeof expected==='function')ok(error instanceof expected);
 else if(expected)for(const [key,value] of Object.entries(expected))strictEqual(error[key],value);
}

// streams-error-edge-cases-test.js:19
const errorDuringPartialConsumption = {
  async test() {
    let chunkCount = 0;

    const rs = new ReadableStream({
      pull(controller) {
        chunkCount++;
        if (chunkCount <= 3) {
          controller.enqueue(chunkCount);
        } else {
          controller.error(new Error('Error after 3 chunks'));
        }
      },
    });

    const reader = rs.getReader();
    const chunks = [];

    for (let i = 0; i < 3; i++) {
      const { value, done } = await reader.read();
      ok(!done);
      chunks.push(value);
    }

    deepStrictEqual(chunks, [1, 2, 3]);

    await rejects(reader.read(), { message: 'Error after 3 chunks' });

    await rejects(reader.read(), { message: 'Error after 3 chunks' });
  },
};

// streams-error-edge-cases-test.js:53
const errorRaceWithCloseReadable = {
  async test() {
    let controller;

    const rs = new ReadableStream({
      start(c) {
        controller = c;
      },
    });

    const reader = rs.getReader();
    const readPromise = reader.read();

    controller.error(new Error('Error wins'));
    try {
      controller.close();
    } catch (_e) {
      // May throw since stream is already errored
    }

    await rejects(readPromise, { message: 'Error wins' });
  },
};

// streams-error-edge-cases-test.js:79
const errorPropagationTeeMultiBranch = {
  async test() {
    let controller;

    const rs = new ReadableStream({
      start(c) {
        controller = c;
      },
    });

    // Create nested tees: original -> [branch1, temp] -> [branch2, branch3]
    const [branch1, temp] = rs.tee();
    const [branch2, branch3] = temp.tee();

    const reader1 = branch1.getReader();
    const reader2 = branch2.getReader();
    const reader3 = branch3.getReader();

    // Start reads on all branches
    const read1 = reader1.read();
    const read2 = reader2.read();
    const read3 = reader3.read();

    // Error the source
    controller.error(new Error('Source error'));

    // All branches should receive the error
    const results = await Promise.allSettled([read1, read2, read3]);

    for (const result of results) {
      strictEqual(result.status, 'rejected');
      strictEqual(result.reason.message, 'Source error');
    }
  },
};

// streams-error-edge-cases-test.js:117
const abortSignalDuringPipe = {
  async test() {
    const chunks = [];
    let pullCount = 0;

    const rs = new ReadableStream({
      async pull(controller) {
        pullCount++;
        await scheduler.wait(10);
        if (pullCount <= 10) {
          controller.enqueue(pullCount);
        } else {
          controller.close();
        }
      },
    });

    const ws = new WritableStream({
      write(chunk) {
        chunks.push(chunk);
      },
    });

    const abortController = new AbortController();

    // Start piping
    const pipePromise = rs.pipeTo(ws, { signal: abortController.signal });

    // Wait for some chunks to flow
    await scheduler.wait(50);

    // Abort mid-pipe
    abortController.abort(new Error('User cancelled'));

    // Pipe should reject with an error (type may vary by implementation)
    await rejects(async () => {
      await pipePromise;
    }, Error);

    // Some chunks should have been written
    ok(chunks.length > 0, 'Some chunks written before abort');
    ok(chunks.length < 10, 'Not all chunks written due to abort');
  },
};

// ts-webstreams-test.js:168
const bufferBackedRequestBody = {
  async test() {
    const request = new Request('http://example.org/', {
      method: 'POST',
      body: 'hello world',
    });
    ok(request.body instanceof ReadableStream);
    strictEqual(await request.text(), 'hello world');
  },
};

// ts-webstreams-test.js:179
const bufferBackedBodyReaderRead = {
  async test() {
    // Reading a buffer-backed body from JavaScript drives the TS reader machinery over
    // the native in-memory source.
    const response = new Response('hello world');
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let text = '';
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      text += decoder.decode(value, { stream: true });
    }
    text += decoder.decode();
    strictEqual(text, 'hello world');
  },
};

// ts-webstreams-test.js:197
const bufferBackedUrlSearchParamsBody = {
  async test() {
    const response = new Response(new URLSearchParams({ a: '1', b: '2' }));
    ok(response.body instanceof ReadableStream);
    strictEqual(await response.text(), 'a=1&b=2');
  },
};

// ts-webstreams-test.js:205
const bufferBackedEmptyBody = {
  async test() {
    // An empty buffer-backed body closes immediately (expectedLength 0).
    const response = new Response('');
    ok(response.body instanceof ReadableStream);
    strictEqual(await response.text(), '');
  },
};

// ts-webstreams-test.js:441
const tsStreamIntoResponse = {
  async test() {
    const encoder = new TextEncoder();
    const rs = new ReadableStream({
      start(c) {
        c.enqueue(encoder.encode('hello '));
        c.enqueue(encoder.encode('world'));
        c.close();
      },
    });
    const response = new Response(rs);
    strictEqual(await response.text(), 'hello world');
  },
};

// ts-webstreams-test.js:510
const tsStreamRequestClone = {
  async test() {
    const rs = new ReadableStream({
      start(c) {
        c.enqueue(new TextEncoder().encode('request body'));
        c.close();
      },
    });
    const request = new Request('http://example.org/', {
      method: 'POST',
      body: rs,
    });
    const clone = request.clone();
    const [a, b] = await Promise.all([request.text(), clone.text()]);
    strictEqual(a, 'request body');
    strictEqual(b, 'request body');
  },
};

// ts-webstreams-test.js:368
const requestBodyProxying = {
  async test() {
    const req1 = new Request('http://example.org/', {
      method: 'POST',
      body: 'hello world',
    });
    const req2 = new Request(req1);
    ok(req2.body instanceof ReadableStream);
    // The original's body has been taken over: disturbed (bodyUsed) and locked.
    strictEqual(req1.bodyUsed, true);
    ok(req1.body.locked);
    strictEqual(await req2.text(), 'hello world');
  },
};

// ts-webstreams-test.js:383
const requestStreamBodyProxying = {
  async test() {
    const encoder = new TextEncoder();
    const rs = new ReadableStream({
      start(c) {
        c.enqueue(encoder.encode('hello '));
        c.enqueue(encoder.encode('world'));
        c.close();
      },
    });
    const req1 = new Request('http://example.org/', {
      method: 'POST',
      body: rs,
    });
    const req2 = new Request(req1);
    // The adopted user stream is the one left as the locked husk...
    ok(rs.locked);
    // ...while the new request reads its (queued) data through the moved cursor.
    strictEqual(await req2.text(), 'hello world');
  },
};

// abortable-fetch-test.js:86
const requestAbortSignal = {
  test() {
    // The request object has an AbortSignal, even if never used.
    const req1 = new Request('');
    ok(Reflect.has(req1, 'signal'));

    // request.signal should be the one passed in.
    const ac = new AbortController();
    const req2 = new Request('', { signal: ac.signal });
    strictEqual(req2.signal, ac.signal);
  },
};

// abortable-fetch-test.js:9
const abortControllerAlreadyAborted = {
  async test(ctrl, env) {
    const ac = new AbortController();
    ac.abort();
    try {
      const _result = await env.subrequest.fetch('http://example.org', {
        signal: ac.signal,
      });
      throw new Error('should have failed');
    } catch (err) {
      strictEqual(err.message, 'The operation was aborted');
    }
  },
};

// abortable-fetch-test.js:48
const abortControllerSyncAbort = {
  async test(ctrl, env) {
    const ac = new AbortController();
    try {
      const promise = env.subrequest.fetch('http://example.org', {
        signal: ac.signal,
      });
      ac.abort();
      await promise;
      throw new Error('should have failed');
    } catch (err) {
      strictEqual(err.message, 'The operation was aborted');
    }
  },
};

// ts-webstreams-test.js:428
const nativeBackedPumpTo = {
  async test(ctrl, env) {
    const response = await env.SELF.fetch('http://example.org/proxy');
    strictEqual(response.status, 200);
    strictEqual(await response.text(), 'hello world');
  },
};

// ts-webstreams-test.js:459
const tsStreamIntoFetchBody = {
  async test(ctrl, env) {
    const rs = new ReadableStream({
      start(c) {
        c.enqueue(new TextEncoder().encode('ping'));
        c.close();
      },
    });
    const response = await env.SELF.fetch('http://example.org/echo', {
      method: 'POST',
      body: rs,
    });
    strictEqual(await response.text(), 'ping');
  },
};

// ts-webstreams-test.js:499
const nativeBackedBodyClone = {
  async test(ctrl, env) {
    const response = await env.SELF.fetch('http://example.org/plain');
    const clone = response.clone();
    const [a, b] = await Promise.all([response.text(), clone.text()]);
    strictEqual(a, 'hello world');
    strictEqual(b, 'hello world');
  },
};
export default {async fetch(request){
const url=new URL(request.url);
if(url.pathname.endsWith('/plain'))return new Response('hello world');
if(url.pathname.endsWith('/echo'))return new Response(request.body);
if(url.pathname.endsWith('/proxy'))return fetch(url.origin+'/workerd/plain');
const local={SELF:{fetch:(input,init)=>fetch(url.origin+'/workerd'+new URL(input).pathname,init)},subrequest:{fetch:(_input,init)=>fetch(url.origin+'/workerd/plain',init)}};

await errorDuringPartialConsumption.test();
await errorRaceWithCloseReadable.test();
await errorPropagationTeeMultiBranch.test();
await abortSignalDuringPipe.test();
await bufferBackedRequestBody.test();
await bufferBackedBodyReaderRead.test();
await bufferBackedUrlSearchParamsBody.test();
await bufferBackedEmptyBody.test();
await tsStreamIntoResponse.test();
await tsStreamRequestClone.test();
await requestBodyProxying.test();
await requestStreamBodyProxying.test();
await requestAbortSignal.test();
await abortControllerAlreadyAborted.test(null,local);
await abortControllerSyncAbort.test(null,local);
await nativeBackedPumpTo.test(null,local);
await tsStreamIntoFetchBody.test(null,local);
await nativeBackedBodyClone.test(null,local);
return Response.json({passed:18});}};
