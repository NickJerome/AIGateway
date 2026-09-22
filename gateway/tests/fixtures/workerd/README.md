# workerd ported test fixtures

Source: [cloudflare/workerd](https://github.com/cloudflare/workerd),
`src/workerd/api/tests/{url,encoding,abortsignal,http,fetch}-test.js`
(Apache-2.0), vendored copies kept at `/data/tmp/m4-verify/workerd-src/`
during the M4 Task 3 port.

Each fixture is a self-contained plugin (`export default { async fetch() }`)
whose body replays the ported workerd assertions. The host harness
(`run_workerd` in `tests/web_api.rs`) prepends `node-assert-shim.mjs` and
drives the plugin down the M2 async request path; a failed assertion throws
in JS, rejects the entry promise, and fails the host test with the JS error
text. Every ported section carries a `// workerd: <file>:<start>-<end>`
provenance comment. The shim is test-only fixture code and is never
referenced from `src/`.

## node-assert-shim.mjs

`node:assert` subset used by the ported assertions: `strictEqual`
(SameValue), `deepStrictEqual` (arrays / Uint8Array / plain objects,
recursive), `ok`, `throws` (bare, RegExp-on-message, constructor, or
`{name, message}` expectation object), `match`, and `fail`. Failures throw
an `Error` with a descriptive message.

## Port / exclusion table

### url-test.js (7251 lines) → `url.js`

Ported (pure parse/serialize semantics, no network, no platform dependence):

| workerd export | Source lines | Fixture section |
| --- | --- | --- |
| `constructAndGet` | 6–141 | ported in full |
| `constructSetAndGet` | 143–293 | ported in full |
| `urlSearchParamsStringifyBehavior` | 295–302 | ported in full |
| `urlSearchParamsForEach` | 304–350 | ported in full |
| `urlSearchParamsInit1` | 352–358 | ported in full |
| `urlToJson` | 409–414 | ported in full |
| `urlSearchParamsSet` | 416–430 | ported in full |
| `urlConstructorTests` | 432–6896 | 50 of 497 table cases (lines 436–450, 479–717, 1018–1137, 1243–1287, 1318–1397, 1398–1491, 6850–6864) + the verbatim assertion loop 6873–6896 |
| `wptTestURLSearchParamsConstructor` | 6898–7096 | all blocks except 6914–6920 and 7087–7094 |
| `urlSetSearch` | 7098–7108 | ported in full |
| `urlSegfaultRegression` | 7110–7120 | ported in full |
| `urlParseStatic` | 7201–7216 | ported in full |
| `urlSearchParamsPipe` | 7218–7224 | ported in full |

Excluded:

| Source lines | Reason |
| --- | --- |
| `urlConstructorTests` 451–478 (`C\|/` drive-letter cases) | Current WHATWG file: host retention rule; rust-url (this runtime's parser) follows the older rule and drops the host. Fixing means replacing the parser — outside spec 01 scope. Deviation registered below. |
| `urlConstructorTests` remaining 445 cases | Subset port per plan; the unported cases are the same shape as the ported ones (table data through the same assertion loop). |
| `urlSearchParamsInit2`/`Init3` (360–386) | Instance-level `Symbol.iterator` override on init objects: generic iterable init is a documented runtime limitation ("Arbitrary iterables are unsupported" in `src/web/url.rs`). |
| `urlSearchParamsInit4`/`Init5` (388–407) | Require `FormData`, which this runtime does not implement. |
| `wptTestURLSearchParamsConstructor` 6914–6920 | Requires the `DOMException` global with its legacy code constants; this runtime has no DOMException. |
| `wptTestURLSearchParamsConstructor` 7087–7094 | Instance-level `Symbol.iterator` override (same limitation as Init2/Init3). |
| `urlPatternBasics` / `urlPatternFun` / `urlPatternExecWithNestedRegexGroups` (7124–7251) | `URLPattern` is not part of spec 01's closed API list. |

No assertion was rewritten to pass, and no workerd error-message wording was
relied upon (the ported `failure: true` cases use bare `throws()`, which only
asserts that construction throws).

### encoding-test.js (1115 lines) → `encoding.js`

Ported (UTF-8 only, per plan):

| workerd export | Source lines | Fixture section |
| --- | --- | --- |
| `decodeStreaming` helper | 9–19 | ported verbatim |
| `utf8Labels` | 42 | ported verbatim |
| `decodeStreamingTest` | 44–178 | UTF-8 subset: 54–112, 125–159 |
| `textEncoderTest` | 180–190 | ported verbatim |
| `encodeWptTest` | 192–385 | only `w3cTestEncode` (197–238) |

Excluded:

| Source lines | Reason |
| --- | --- |
| `windows1252Labels` (22–40) and decodeStreamingTest 48–53 | windows-1252 is a non-UTF-8 encoding; this runtime is UTF-8-only (spec 01). |
| decodeStreamingTest 114–123 (ascii) | `ascii` is a windows-1252 label — non-UTF-8. |
| decodeStreamingTest 161–176 (shift_jis) | Non-UTF-8 encoding. |
| `encodeWptTest` `w3cTestEncodeInto` (240–383) | `TextEncoder.encodeInto` is not implemented in this runtime. |
| `big5` (387–403), `utf16leFastTrack` (405–416), `allTheDecoders` (418+), gbk / iso2022jp / x-user-defined sections | Non-UTF-8 encodings. |
| All `TextDecoderStream` / `TextEncoderStream` tests | Streams belong to spec 02. |

### abortsignal-test.js (1102 lines) → `abortsignal.js`

Ported (per plan):

| workerd export | Source lines | Fixture section |
| --- | --- | --- |
| `abortcontroller` | 184–233 | subset: 189–197, 208–218, 223–228 |
| `abortcontrollerWithReason` | 234–254 | ported verbatim (234–253) |

Excluded:

| Source lines | Reason |
| --- | --- |
| 186–187 (`new AbortSignal()` throws) | This runtime deliberately allows `new AbortSignal()` — long-standing registered deviation (M3a). The assertion is dropped, not rewritten. |
| 199–203, 230 (`onabort` handler / count) | The `onabort` property is not implemented (excluded per plan). |
| 205–206, 220–221 (`throwIfAborted`) | Not implemented (excluded per plan). |
| `alreadyAborted` and everything after 254 | `AbortSignal.abort()` static, `timeout()`, `any()`, rpc — all excluded per plan. |

### http-test.js (445 lines) + fetch-test.js (126 lines) → `http.js`

Ported (no-network Request/Response/Headers constructor semantics only):

| workerd export | Source lines | Fixture section |
| --- | --- | --- |
| `fetchGen` | 52–54 | plain string BodyInit round trip |

Excluded:

| Source lines | Reason |
| --- | --- |
| http-test.js default `fetch`/`scheduled`/`test` exports (10–117) | All bound to `env.SERVICE.fetch`/`scheduled` bindings — network/workerd-runtime integration. |
| http-test.js `test` inspect section (120–292) | Asserts `util.inspect` formatting, `FormData`, `Blob`, `File`, `WebSocketPair` — none implemented; also network-bound. |
| http-test.js `cacheMode` (294–445) | `Request.prototype.cache` is workerd-specific (depends on `env.CACHE_ENABLED`) and fetch-network-bound. |
| fetch-test.js `test` (12–27) | Depends on the `upper_case_all_http_methods` compatibility flag (`globalThis.Cloudflare.compatibilityFlags`) — workerd-specific. |
| fetch-test.js `fetchGen` 29–51, 56–126 | Generator / array / sync+async iterator BodyInit forms are Streams-era BodyInit extensions not implemented in this runtime. |
| fetch-test.js `fetchGen` 48–50 (boxed `new String`) | Web IDL union fallback stringifies arbitrary objects; this runtime deliberately pins `TypeError` for non-BodyInit objects (see `unsupported_body_init_types_throw` in `tests/web_api.rs`). Dropped, not rewritten. |

## Deviation register (this port)

1. **file: URL drive-letter host retention** — rust-url drops the host for
   `C|/` inputs where the current WHATWG URL Standard keeps it
   (`file://host/C:/`). The two workerd cases are excluded (url-test.js
   451–478) rather than weakening the assertions.
2. **Generic iterable `URLSearchParams` init** — instance-level
   `Symbol.iterator` overrides on the init object are ignored (documented
   runtime limitation). urlSearchParamsInit2/Init3 and one
   wptTestURLSearchParamsConstructor block excluded.
3. **`new AbortSignal()` is constructible** — pre-existing registered
   deviation (M3a); the `throws(() => new AbortSignal())` line is excluded.
4. **BOM sniffing re-arms per flush cycle** — discovered during the port:
   the runtime stripped a BOM only once per `TextDecoder` instance. Per the
   Encoding Standard (`decode()` step 1 resets `BOM seen` when the previous
   call flushed) and Node's behavior, this was a runtime bug; fixed in
   `src/web/text.rs` (assertions untouched).
5. **`URLSearchParams.forEach` liveness and `thisArg`** — discovered during
   the port: the runtime iterated a snapshot and ignored the second argument.
   Both are observable per the URL Standard and workerd's test; fixed in
   `src/web/url.rs` (assertions untouched).
6. **`host` setter port suffix** — discovered during the port: `u.host =
   'foo.example.com:80'` dropped the port instead of adopting it. Fixed in
   `src/web/url.rs` per the WHATWG host setter (assertions untouched).


## Spec 02 port (2026-09-21)

`spec02.js` selects 18 assertion groups from workerd commit
`3ea9117c64c9fd29d734a60e066ef2872ee479aa` at
https://github.com/cloudflare/workerd/tree/3ea9117c64c9fd29d734a60e066ef2872ee479aa/src/workerd/api/tests .
Each copied group has a source filename and starting line. The Apache-2.0
notice is preserved; the license is in `workerd.LICENSE`.

- `streams-error-edge-cases-test.js`: all four groups (partial errors,
  close/error race, nested tee errors, AbortSignal during pipeTo).
- `ts-webstreams-test.js`: buffer Request/reader/URLSearchParams/empty body,
  stream Response, Request clone, Request adoption (buffer/stream), native
  pump-through, stream upload and native Response clone.
- `abortable-fetch-test.js`: Request signal, pre-aborted fetch and synchronous
  fetch abort.

The test harness replaces `scheduler.wait` with setTimeout, provides a
small test-only `rejects` assertion and maps workerd SELF/subrequest bindings
to a local HTTP gateway. Assertions are not weakened to match the runtime.
The HTTP constructor check from the earlier `http.js` fixture is retained;
real HTTP, redirects, TLS and proxy semantics have dedicated Rust fixtures.

Excluded: BYOB/minimum-byte reads/byte controllers; Blob-based cases;
iterable BodyInit, IdentityTransformStream/FixedLengthStream and compatibility
flag introspection; AbortSignal.timeout/abort statics and service-RPC tests.
The entire upstream/WPT suite has not been run.

## Spec 11 port (2026-09-21)

`spec11.js` ports 15 unchanged assertion groups from the local workerd checkout
at commit `1ed4b65ccdf861023a82f9639fd2d6d8d01cd67e`:

- `websocket-constructor-test.js`: empty protocols, string protocol, invalid/
  duplicate tokens, UTF-8 close length, CloseEvent USVString, valid and invalid
  client close codes, and ArrayBuffer messages (12 groups).
- `events-test.js`: MessageEvent, CloseEvent and ErrorEvent initialization
  (3 groups).

Each group has its original source line. The test-only adapter supplies
`doesNotThrow` and replaces example.com with local port 9, so constructor tests
cannot contact a real service. Original assertions and error-name checks remain.
License: Apache-2.0, `workerd.LICENSE`.

`tests/websocket.rs` additionally adapts the network scenarios from
`http-test.js` and `websocket-client-error-test.js`: local HTTP/WS replaces
service/sidecar bindings; the oversized message is 2048 bytes with a 1024-byte
configured limit, rather than workerd's 33/32 MiB fixture. It checks the error
then close(1009), without asserting workerd's C++-specific error wording.
HTTP inspect formatting is not copied (there is no node:util in the product).

Excluded: Blob and binaryType switching, global error event fail-fast policy,
compression, half-open behavior, hibernation and complete WPT. Client
constructors use workerd's selected strict close-code validation; accepted
server endpoints additionally permit protocol close codes for relaying peers.
These exclusions are not counted as passing workerd tests.
