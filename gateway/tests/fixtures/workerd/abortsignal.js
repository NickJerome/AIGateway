// Ported assertions from workerd's src/workerd/api/tests/abortsignal-test.js
// (cloudflare/workerd, Apache-2.0). Ported: abortcontroller (L184-233 subset)
// and abortcontrollerWithReason (L234-253). Excluded: the `onabort` property
// lines, `throwIfAborted`, `AbortSignal.abort()` static, `timeout()`,
// `any()`, and all rpc sections — see README.md in this directory.
// Every section carries a `// workerd: <file>:<start>-<end>` provenance mark.
//
// The host harness prepends node-assert-shim.mjs, which provides
// strictEqual / deepStrictEqual / ok / throws / match / fail.

export default {
  async fetch() {
    // workerd: abortsignal-test.js:184-232 (abortcontroller, subset)
    {
      // Excluded: L186-187 (`new AbortSignal()` throws). This runtime
      // deliberately allows constructing AbortSignal (registered deviation).

      // workerd: abortsignal-test.js:189-194
      const ac = new AbortController();
      ok(ac.signal instanceof AbortSignal);
      strictEqual(ac.signal.aborted, false);

      // every call to ac.signal should always be the same value.
      strictEqual(ac.signal, ac.signal);

      // workerd: abortsignal-test.js:196-197 (signal is read only; ESM is
      // strict mode, so assigning to the getter-only accessor throws)
      throws(() => (ac.signal = 1));

      // Excluded: L199-203 (onabort handler) and L205-206 (throwIfAborted on
      // a non-aborted signal) — neither is implemented in this runtime.

      // workerd: abortsignal-test.js:208-210 (reason and aborted are read only)
      throws(() => (ac.signal.reason = 1));
      throws(() => (ac.signal.aborted = 'foo'));

      // workerd: abortsignal-test.js:212-218
      // trigger our abort with a default reason...
      ac.abort();

      // This one shouldn't get called since it is added after the abort
      ac.signal.addEventListener('abort', () => {
        throw new Error('should not have been called');
      });

      // Excluded: L220-221 (throwIfAborted on an aborted signal).

      // workerd: abortsignal-test.js:223-228
      strictEqual(ac.signal.aborted, true);
      strictEqual(ac.signal.reason.message, 'The operation was aborted');
      strictEqual(ac.signal.reason.name, 'AbortError');

      // Abort can be called multiple times with no effect.
      ac.abort();

      // Excluded: L230 (onabort invocation count) — onabort is excluded.
    }

    // workerd: abortsignal-test.js:234-253 (abortcontrollerWithReason, verbatim)
    {
      const ac = new AbortController();
      ok(ac.signal instanceof AbortSignal);
      strictEqual(ac.signal.aborted, false);

      let invoked = 0;

      ac.signal.addEventListener('abort', (event) => {
        invoked++;
        strictEqual(ac.signal.reason, 'foo');
      });

      ac.abort('foo');
      strictEqual(ac.signal.aborted, true);
      strictEqual(ac.signal.reason, 'foo');

      strictEqual(invoked, 1);
    }

    return 'workerd-abortsignal ok';
  },
};
