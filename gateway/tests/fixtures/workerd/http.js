// Ported assertions from workerd's src/workerd/api/tests/http-test.js and
// fetch-test.js (cloudflare/workerd, Apache-2.0). Only no-network
// Request/Response constructor semantics are ported. Excluded: every fetch
// network case, util.inspect formatting, WebSocket, scheduled(), Request
// cache mode, the upper_case_all_http_methods compatibility flag, and all
// iterable/async-iterator BodyInit forms (Streams-era capability not in this
// runtime). See README.md in this directory for the full table.
// Every section carries a `// workerd: <file>:<start>-<end>` provenance mark.
//
// The host harness prepends node-assert-shim.mjs, which provides
// strictEqual / deepStrictEqual / ok / throws / match / fail.

export default {
  async fetch() {
    // http-test.js contributes no portable no-network assertions: its tests
    // are all bound to env.SERVICE.fetch/scheduled, util.inspect output or
    // WebSocketPair. The single portable constructor-semantics assertion
    // comes from fetch-test.js.

    // workerd: fetch-test.js:52-54 (fetchGen: plain string BodyInit)
    {
      // Regular strings work correctly.
      const resp4 = new Response('Hello');
      strictEqual(await resp4.text(), 'Hello');
    }

    return 'workerd-http ok';
  },
};
