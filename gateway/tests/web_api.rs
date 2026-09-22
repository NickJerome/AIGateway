//! workord-style assertions for the foundation Web API objects (spec 01
//! "基础 API"): Headers, URL/URLSearchParams, TextEncoder/TextDecoder,
//! Event/EventTarget, AbortController/AbortSignal.

use gateway::bytecode::compile;
use gateway::engine::Invocation;
use gateway::worker::Worker;

/// Assertion helpers injected into every test plugin.
const ASSERTS: &str = r#"
function assert(cond, msg) { if (!cond) throw new Error('assert failed: ' + msg); }
function assertEq(actual, expected, msg) {
    if (actual !== expected) {
        throw new Error(msg + ': expected ' + JSON.stringify(expected) + ', got ' + JSON.stringify(actual));
    }
}
"#;

/// Run a synchronous fetch() body against a fresh request Engine and return
/// its string result. Assertion failures surface as Err from `invoke`.
fn run(body: &str) -> String {
    let source = format!("{ASSERTS}\nexport default {{ fetch() {{ {body} \n}} }};");
    let code = compile("plugin.mjs", &source).unwrap().code;
    Worker::new()
        .start_request()
        .unwrap()
        .invoke(
            &code,
            Invocation {
                url: "/".into(),
                method: "GET".into(),
            },
        )
        .unwrap()
}

// --- Headers ---

#[test]
fn headers_get_is_case_insensitive_and_combines() {
    run(r#"
const h = new Headers();
h.append('X-Thing', '1');
h.append('x-thing', '2');
assertEq(h.get('X-THING'), '1, 2', 'combined get');
assertEq(h.get('missing'), null, 'missing get is null');
assert(h.has('X-THING'), 'has is case insensitive');
return 'ok';
"#);
}

#[test]
fn headers_set_cookie_is_never_combined() {
    run(r#"
const h = new Headers();
h.append('Set-Cookie', 'a=1; Path=/');
h.append('set-cookie', 'b=2; Path=/');
assertEq(h.get('set-cookie'), 'a=1; Path=/', 'get returns the first Set-Cookie only');
assertEq(JSON.stringify(h.getSetCookie()), '["a=1; Path=/","b=2; Path=/"]', 'getSetCookie');
h.set('Set-Cookie', 'c=3');
assertEq(JSON.stringify(h.getSetCookie()), '["c=3"]', 'set replaces all Set-Cookie');
return 'ok';
"#);
}

#[test]
fn headers_iteration_is_sorted_and_stable() {
    run(r#"
const h = new Headers();
h.append('b', '2');
h.append('Set-Cookie', 'c1');
h.append('a', '1');
h.append('b', '3');
h.append('set-cookie', 'c2');
const entries = [];
for (const [k, v] of h.entries()) entries.push(k + '=' + v);
assertEq(JSON.stringify(entries), '["a=1","b=2, 3","set-cookie=c1","set-cookie=c2"]', 'sorted, combined, set-cookie split');
assertEq(JSON.stringify([...h.keys()]), '["a","b","set-cookie","set-cookie"]', 'keys');
assertEq(JSON.stringify([...h.values()]), '["1","2, 3","c1","c2"]', 'values');
return 'ok';
"#);
}

#[test]
fn headers_set_and_delete() {
    run(r#"
const h = new Headers();
h.append('x-dup', '1');
h.append('a-first', '0');
h.append('x-dup', '2');
h.set('x-dup', '3');
assertEq(h.get('x-dup'), '3', 'set replaces every duplicate');
assertEq(JSON.stringify([...h.keys()]), '["a-first","x-dup"]', 'set keeps the first position');
h.delete('X-Dup');
assert(!h.has('x-dup'), 'delete removes the name');
assertEq(h.get('x-dup'), null, 'deleted get is null');
return 'ok';
"#);
}

#[test]
fn headers_construct_from_record_array_and_clone() {
    run(r#"
const fromRecord = new Headers({ 'X-A': '1' });
assertEq(fromRecord.get('x-a'), '1', 'record init');
const fromPairs = new Headers([['x-b', '1'], ['x-b', '2']]);
assertEq(fromPairs.get('x-b'), '1, 2', 'pair array init keeps duplicates');
const cloned = new Headers(fromPairs);
assertEq(cloned.get('X-B'), '1, 2', 'clone from Headers instance');
cloned.append('x-b', '3');
assertEq(fromPairs.get('x-b'), '1, 2', 'clone is independent');
assertEq(new Headers().get('x'), null, 'empty init');
return 'ok';
"#);
}

#[test]
fn headers_for_each_passes_value_key_and_headers() {
    run(r#"
const h = new Headers([['b', '2'], ['a', '1']]);
const seen = [];
h.forEach((value, key, self) => {
    assert(self === h, 'forEach third argument is the Headers');
    seen.push(key + ':' + value);
});
assertEq(JSON.stringify(seen), '["a:1","b:2"]', 'forEach sorted by name');
return 'ok';
"#);
}

/// Run a fetch() body expected to throw; returns the error text.
fn run_err(body: &str) -> String {
    let source = format!("{ASSERTS}\nexport default {{ fetch() {{ {body} \n}} }};");
    let code = compile("plugin.mjs", &source).unwrap().code;
    let err = Worker::new()
        .start_request()
        .unwrap()
        .invoke(
            &code,
            Invocation {
                url: "/".into(),
                method: "GET".into(),
            },
        )
        .unwrap_err();
    format!("{err:#}")
}

// --- URL ---

#[test]
fn url_parses_absolute_components() {
    run(r#"
const u = new URL('https://user:pw@example.com:8443/a/b?x=1#frag');
assertEq(u.href, 'https://user:pw@example.com:8443/a/b?x=1#frag', 'href');
assertEq(u.protocol, 'https:', 'protocol');
assertEq(u.username, 'user', 'username');
assertEq(u.password, 'pw', 'password');
assertEq(u.host, 'example.com:8443', 'host');
assertEq(u.hostname, 'example.com', 'hostname');
assertEq(u.port, '8443', 'port');
assertEq(u.pathname, '/a/b', 'pathname');
assertEq(u.search, '?x=1', 'search');
assertEq(u.hash, '#frag', 'hash');
assertEq(u.origin, 'https://example.com:8443', 'origin');
assertEq(u.toString(), u.href, 'toString');
assertEq(u.toJSON(), u.href, 'toJSON');
return 'ok';
"#);
}

#[test]
fn url_resolves_relative_against_base() {
    run(r#"
const u = new URL('../d?y=2', 'https://example.com/a/b/c');
assertEq(u.href, 'https://example.com/a/d?y=2', 'relative resolution');
assert(URL.canParse('../d', 'https://example.com/a/b'), 'canParse true');
assert(!URL.canParse('http://[::1', undefined), 'canParse false');
const parsed = URL.parse('//example.org/x', 'https://example.com/');
assertEq(parsed.href, 'https://example.org/x', 'URL.parse protocol-relative');
assertEq(URL.parse(':::bad'), null, 'URL.parse failure is null');
return 'ok';
"#);
}

#[test]
fn url_setters_update_href() {
    run(r#"
const u = new URL('https://example.com/a?x=1#f');
u.pathname = '/b c';
assert(u.href.startsWith('https://example.com/b%20c'), 'pathname is encoded: ' + u.href);
u.search = '?y=2';
assertEq(u.search, '?y=2', 'search setter');
u.hash = 'g';
assertEq(u.hash, '#g', 'hash setter');
u.port = '9090';
assertEq(u.host, 'example.com:9090', 'port setter');
u.protocol = 'http:';
assertEq(u.href, 'http://example.com:9090/b%20c?y=2#g', 'href after setters');
u.port = '';
assertEq(u.host, 'example.com', 'empty port clears');
return 'ok';
"#);
}

#[test]
fn url_search_params_are_live_in_both_directions() {
    run(r#"
const u = new URL('https://example.com/?a=1&a=2');
const p = u.searchParams;
assert(u.searchParams === p, 'searchParams identity is stable');
assertEq(p.get('a'), '1', 'get first');
assertEq(JSON.stringify(p.getAll('a')), '["1","2"]', 'getAll');
p.append('b', '3');
assertEq(u.search, '?a=1&a=2&b=3', 'params mutation writes the URL');
u.search = '?z=9';
assertEq(p.get('z'), '9', 'URL mutation resyncs the params');
assert(!p.has('a'), 'old pairs are gone after resync');
return 'ok';
"#);
}

#[test]
fn url_invalid_input_and_base_throw_type_error() {
    let err = run_err(
        r#"
try { new URL(':::not a url'); } catch (e) {
    assert(e instanceof TypeError, 'invalid URL throws TypeError, got ' + e);
    throw new Error('caught:' + (e instanceof TypeError));
}
return 'unreachable';
"#,
    );
    assert!(err.contains("caught:true"), "{err}");
    // An illegal base throws even when the input is absolute (browser/rust-url).
    let err = run_err("new URL('https://absolute.example/', ':::bad base'); return 'unreachable';");
    assert!(err.contains("Invalid URL"), "{err}");
}

// --- URLSearchParams ---

#[test]
fn params_string_init_decodes_repeats_and_encoding() {
    run(r#"
const p = new URLSearchParams('?a=1&a=2&b=%E4%B8%AD&c=a+b');
assertEq(JSON.stringify(p.getAll('a')), '["1","2"]', 'repeated names');
assertEq(p.get('b'), '中', 'percent decoding');
assertEq(p.get('c'), 'a b', '+ decodes to space');
assertEq(p.size, 4, 'size');
return 'ok';
"#);
}

#[test]
fn params_mutation_and_iteration() {
    run(r#"
const p = new URLSearchParams([['b', '2'], ['a', '1'], ['b', '3']]);
p.delete('b', '2');
assertEq(JSON.stringify(p.getAll('b')), '["3"]', 'delete with value');
p.set('a', '9');
assertEq(p.get('a'), '9', 'set');
const beforeSort = [];
p.forEach((v, k) => beforeSort.push(k));
assertEq(JSON.stringify(beforeSort), '["a","b"]', 'set replaces in place');
p.sort();
const entries = [];
p.forEach((v, k) => entries.push(k + '=' + v));
assertEq(JSON.stringify(entries), '["a=9","b=3"]', 'sort is stable by name');
assertEq(JSON.stringify([...p.keys()]), '["a","b"]', 'keys');
assertEq(JSON.stringify([...p.values()]), '["9","3"]', 'values');
return 'ok';
"#);
}

#[test]
fn params_to_string_and_record_init() {
    run(r#"
const p = new URLSearchParams({ q: 'a b', lang: '中' });
const s = p.toString();
assert(s === 'q=a+b&lang=%E4%B8%AD', 'form urlencoded: ' + s);
assertEq(new URLSearchParams(s).get('lang'), '中', 'round trip');
return 'ok';
"#);
}

// --- TextEncoder / TextDecoder ---

#[test]
fn encoder_encodes_utf8_bytes() {
    run(r#"
const enc = new TextEncoder();
assertEq(enc.encoding, 'utf-8', 'encoding label');
assertEq(JSON.stringify([...enc.encode('Aé中')]), '[65,195,169,228,184,173]', 'utf-8 bytes');
assertEq(enc.encode('').length, 0, 'empty');
assertEq(enc.encode(undefined).length, 0, 'undefined encodes empty');
assertEq(JSON.stringify([...enc.encode(42)]), '[52,50]', 'input is coerced to string');
assertEq(typeof enc.encodeInto, 'undefined', 'encodeInto is not provided');
return 'ok';
"#);
}

#[test]
fn encoder_replaces_lone_surrogates_and_merges_pairs() {
    run(r#"
const enc = new TextEncoder();
// A surrogate pair encodes as the combined 4-byte UTF-8 sequence.
assertEq(JSON.stringify([...enc.encode('😀')]), '[240,159,152,128]', 'surrogate pair');
// A lone surrogate becomes U+FFFD; String::to_string() would have thrown.
assertEq(JSON.stringify([...enc.encode('\uD800')]), '[239,191,189]', 'lone high surrogate');
assertEq(JSON.stringify([...enc.encode('x\uDC00y')]), '[120,239,191,189,121]', 'lone low surrogate');
return 'ok';
"#);
}

#[test]
fn decoder_decodes_chunks_with_streaming_state() {
    run(r#"
const dec = new TextDecoder();
assertEq(dec.encoding, 'utf-8', 'encoding label');
assertEq(dec.fatal, false, 'fatal default');
const bytes = new Uint8Array([228, 184, 173, 229, 165, 189]); // 你好
// Split inside the first 3-byte sequence: the tail stays pending.
assertEq(dec.decode(bytes.subarray(0, 1), { stream: true }), '', 'incomplete lead held');
assertEq(dec.decode(bytes.subarray(1, 3), { stream: true }), '中', 'first char completes');
assertEq(dec.decode(bytes.subarray(3, 5), { stream: true }), '', 'second char held');
assertEq(dec.decode(bytes.subarray(5)), '好', 'final chunk completes without stream');
return 'ok';
"#);
}

#[test]
fn decoder_fatal_and_ignore_bom() {
    let err = run_err(
        r#"
const dec = new TextDecoder('utf-8', { fatal: true });
try { dec.decode(new Uint8Array([0xFF])); } catch (e) {
    throw new Error('caught:' + (e instanceof TypeError));
}
return 'unreachable';
"#,
    );
    assert!(err.contains("caught:true"), "{err}");
    // A truncated tail at end-of-stream is also fatal.
    let err = run_err(
        "new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([228, 184])); return 'unreachable';",
    );
    assert!(err.contains("not valid"), "{err}");
    run(r#"
const keep = new TextDecoder('utf-8', { ignoreBOM: true });
assertEq(keep.decode(new Uint8Array([0xEF, 0xBB, 0xBF, 65])), '\uFEFFA', 'ignoreBOM keeps U+FEFF');
assertEq(new TextDecoder().decode(new Uint8Array([0xEF, 0xBB, 0xBF, 65])), 'A', 'BOM stripped by default');
const bad = new TextDecoder();
assertEq(bad.decode(new Uint8Array([65, 0xFF, 66])), 'A\uFFFD' + 'B', 'invalid byte is one U+FFFD');
// A 2-byte lead followed by an ASCII byte: consume the lead only (standard error length).
assertEq(bad.decode(new Uint8Array([0xC3, 40])), '\uFFFD(', 'bad continuation consumes lead only');
return 'ok';
"#);
}

#[test]
fn decoder_bom_split_across_chunks() {
    run(r#"
const dec = new TextDecoder();
// The BOM prefix is held back until the next chunk disambiguates it.
assertEq(dec.decode(new Uint8Array([0xEF]), { stream: true }), '', 'BOM prefix held');
assertEq(dec.decode(new Uint8Array([0xBB, 0xBF, 65]), { stream: true }), 'A', 'split BOM stripped');
const dec2 = new TextDecoder();
assertEq(dec2.decode(new Uint8Array([0xEF]), { stream: true }), '', 'prefix held again');
assertEq(dec2.decode(new Uint8Array([0x41]), { stream: true }), '\uFFFD' + 'A', 'not a BOM: EF becomes U+FFFD');
return 'ok';
"#);
}

#[test]
fn decoder_rejects_unknown_labels() {
    run(r#"
try { new TextDecoder('utf-16'); throw new Error('no throw'); } catch (e) {
    assert(e instanceof RangeError, 'unknown label is RangeError, got ' + e);
}
const ok = new TextDecoder('UTF8');
assertEq(ok.encoding, 'utf-8', 'utf8 label alias');
return 'ok';
"#);
}

// --- Event / EventTarget ---

#[test]
fn event_target_dispatches_in_registration_order() {
    run(r#"
const t = new EventTarget();
const seen = [];
t.addEventListener('go', (e) => {
    assertEq(e.type, 'go', 'event type');
    assert(e.target === t, 'event target is the dispatching object');
    seen.push('a');
});
t.addEventListener('go', () => seen.push('b'));
t.addEventListener('other', () => seen.push('x'));
const ev = new Event('go', { bubbles: true, cancelable: true });
assertEq(ev.bubbles, true, 'init flag bubbles');
assertEq(ev.cancelable, true, 'init flag cancelable');
assertEq(ev.composed, false, 'composed default');
assertEq(ev.defaultPrevented, false, 'defaultPrevented default');
assertEq(ev.target, null, 'target is null outside dispatch');
assert(t.dispatchEvent(ev), 'dispatch returns true when not canceled');
assertEq(ev.target, null, 'target cleared after dispatch');
assertEq(JSON.stringify(seen), '["a","b"]', 'registration order, other type skipped');
// Duplicate (type, callback) registration is ignored.
const dup = () => seen.push('d');
t.addEventListener('go', dup);
t.addEventListener('go', dup);
t.dispatchEvent(new Event('go'));
assertEq(JSON.stringify(seen), '["a","b","a","b","d"]', 'duplicate ignored');
return 'ok';
"#);
}

#[test]
fn event_once_and_remove_and_reentry() {
    run(r#"
const t = new EventTarget();
const seen = [];
// once: removed BEFORE the callback, so a re-entrant dispatch skips it.
t.addEventListener('go', () => {
    seen.push('once');
    t.dispatchEvent(new Event('go'));
}, { once: true });
t.addEventListener('go', () => {
    seen.push('stable');
    // Removing a not-yet-called listener keeps it in this dispatch? No:
    // dispatch snapshots ids and skips listeners removed before their turn.
    t.removeEventListener('go', late);
});
function late() { seen.push('late'); }
t.addEventListener('go', late);
t.dispatchEvent(new Event('go'));
// First dispatch: once (re-dispatch reenters), stable, late removed -> skipped.
// Re-entrant dispatch: once already gone, stable runs, late removed -> skipped.
assertEq(JSON.stringify(seen), '["once","stable","stable"]', 'got ' + JSON.stringify(seen));
return 'ok';
"#);
}

#[test]
fn event_listener_throw_interrupts_dispatch_and_propagates() {
    // Repair item from the previous round: the plan must prove that a
    // throwing listener aborts the dispatch, propagates the exception, and
    // prevents later listeners from running.
    let err = run_err(
        r#"
const t = new EventTarget();
let laterRan = false;
t.addEventListener('go', () => { throw new Error('listener boom'); });
t.addEventListener('go', () => { laterRan = true; });
try { t.dispatchEvent(new Event('go')); } catch (e) {
    assertEq(e.message, 'listener boom', 'exception propagates');
    assertEq(laterRan, false, 'later listener did not run');
    throw new Error('verified:' + e.message);
}
return 'unreachable';
"#,
    );
    assert!(err.contains("verified:listener boom"), "{err}");
}

#[test]
fn event_prevent_default_controls_dispatch_result() {
    run(r#"
const t = new EventTarget();
t.addEventListener('c', (e) => e.preventDefault());
t.addEventListener('n', (e) => e.preventDefault());
const cancelable = new Event('c', { cancelable: true });
assertEq(t.dispatchEvent(cancelable), false, 'canceled dispatch returns false');
assertEq(cancelable.defaultPrevented, true, 'defaultPrevented set');
const plain = new Event('n');
assertEq(t.dispatchEvent(plain), true, 'non-cancelable preventDefault is a no-op');
assertEq(plain.defaultPrevented, false, 'flag stays unset');
return 'ok';
"#);
}

// --- AbortController / AbortSignal ---

#[test]
fn abort_fires_listeners_once_with_default_reason() {
    run(r#"
const c = new AbortController();
const s = c.signal;
assert(c.signal === s, 'signal identity stable');
assertEq(s.aborted, false, 'not aborted initially');
assertEq(s.reason, undefined, 'reason undefined before abort');
const seen = [];
s.addEventListener('abort', (e) => { seen.push('a'); assertEq(e.type, 'abort', 'abort event'); });
c.abort();
c.abort(); // idempotent: listeners fire exactly once
assertEq(JSON.stringify(seen), '["a"]', 'abort fires once');
assertEq(s.aborted, true, 'aborted flag');
assertEq(s.reason.name, 'AbortError', 'default reason name');
assert(s.reason instanceof Error, 'default reason is an Error (no DOMException)');
return 'ok';
"#);
}

#[test]
fn abort_with_custom_reason_and_listener_removal() {
    run(r#"
const c = new AbortController();
const s = c.signal;
const seen = [];
const removable = () => seen.push('r');
s.addEventListener('abort', removable);
s.removeEventListener('abort', removable);
s.addEventListener('abort', () => seen.push('kept'), { once: true });
c.abort('stop now');
assertEq(JSON.stringify(seen), '["kept"]', 'removed listener skipped');
assertEq(s.reason, 'stop now', 'custom reason kept verbatim');
return 'ok';
"#);
}

#[test]
fn abort_signal_is_constructible_but_not_an_event_target() {
    run(r#"
// Deviation (documented in the plan): browsers throw TypeError on
// `new AbortSignal()`; this runtime allows it. And AbortSignal does not
// inherit the EventTarget prototype chain.
const s = new AbortSignal();
assertEq(s.aborted, false, 'constructible signal');
assert(!(s instanceof EventTarget), 'AbortSignal is not an EventTarget subclass');
assertEq(typeof s.addEventListener, 'function', 'but it carries listener methods');
return 'ok';
"#);
}

// --- Async path: the same objects on the M2 request task ---

use gateway::limits::Limits;
use gateway::runtime::executor::Executor;
use gateway::runtime::request::{Phase, RequestHandle};
use std::time::{Duration, Instant};

fn run_async(body: &str) -> (RequestHandle, String) {
    let worker = Worker::with_limits(Limits {
        slice_ms: 1000,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let source = format!("{ASSERTS}\nexport default {{ async fetch() {{ {body} \n}} }};");
    let code = compile("plugin.mjs", &source).unwrap().code;
    let engine = worker.start_request().unwrap();
    let (handle, task) = engine.into_task(
        &exec.handle(),
        &code,
        Invocation {
            url: "/".into(),
            method: "GET".into(),
        },
    );
    exec.spawn(task.drive());
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            // Stand-in for the 02 HTTP layer.
            let deadline = Instant::now() + Duration::from_secs(5);
            while h.phase() != Phase::Responding {
                assert!(Instant::now() < deadline, "never reached Responding");
                std::thread::sleep(Duration::from_millis(1));
            }
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            body
        })
    };
    exec.run();
    (handle, http.join().unwrap())
}

#[test]
fn web_objects_work_across_timer_suspension() {
    let (handle, body) = run_async(
        r#"
await new Promise(r => setTimeout(r, 30));
const h = new Headers([['X-A', '1']]);
const u = new URL('/p?q=' + encodeURIComponent('中'), 'https://example.com');
const dec = new TextDecoder();
const half = new Uint8Array([228, 184]);
await new Promise(r => setTimeout(r, 30));
const text = dec.decode(half, { stream: true }) + dec.decode(new Uint8Array([173]));
assertEq(text, '中', 'decoder state survives a timer suspension');
return h.get('x-a') + ' ' + u.href;
"#,
    );
    assert_eq!(body, "1 https://example.com/p?q=%E4%B8%AD");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(handle.outcome().unwrap().unwrap(), body);
}

#[test]
fn abort_controller_drives_a_timer_callback() {
    let (handle, body) = run_async(
        r#"
const c = new AbortController();
let fired = 0;
c.signal.addEventListener('abort', () => { fired++; }, { once: true });
setTimeout(() => { c.abort(); c.abort(); }, 20);
await new Promise(r => setTimeout(r, 80));
assertEq(fired, 1, 'abort fired exactly once across timers');
assertEq(c.signal.aborted, true, 'aborted');
assertEq(c.signal.reason.name, 'AbortError', 'default reason');
return 'aborted:' + fired;
"#,
    );
    assert_eq!(body, "aborted:1");
    assert_eq!(handle.phase(), Phase::Closed);
}

// --- Request / Response (spec 01 line 28) ---

#[test]
fn request_constructs_and_exposes_properties() {
    run(r#"
const r = new Request('https://example.com/p?q=1', { method: 'post', headers: { 'X-A': '1' }, body: 'hi' });
assertEq(r.url, 'https://example.com/p?q=1', 'url');
assertEq(r.method, 'POST', 'method is normalized');
assertEq(r.headers.get('x-a'), '1', 'headers init');
assertEq(r.bodyUsed, false, 'bodyUsed starts false');
assert(r.body !== null, 'body handle present');
const g = new Request('https://example.com/');
assertEq(g.method, 'GET', 'default method');
assertEq(g.body, null, 'no body is null');
const c = new Request(r);
assertEq(c.url, r.url, 'request input clones url');
assertEq(c.method, 'POST', 'request input clones normalized method');
assertEq(c.headers.get('x-a'), '1', 'request input clones headers');
return 'ok';
"#);
}

#[test]
fn request_with_get_or_head_rejects_a_body() {
    let err = run_err("new Request('https://x', { body: 'x' }); return 'unreachable';");
    assert!(err.contains("GET/HEAD"), "{err}");
}

#[test]
fn response_constructs_and_validates_status() {
    run(r#"
const r = new Response('hello', { status: 201, headers: { 'X-B': '2' } });
assertEq(r.status, 201, 'status');
assertEq(r.ok, true, 'ok for 2xx');
assertEq(r.headers.get('x-b'), '2', 'headers');
assertEq(r.bodyUsed, false, 'bodyUsed');
assertEq(r.url, '', 'url defaults to empty');
assertEq(new Response(null, { status: 204 }).ok, true, '204 is ok');
assertEq(new Response(null, { status: 404 }).ok, false, '404 is not ok');
let threw = '';
try { new Response('x', { status: 99 }); } catch (e) { threw = e.name; }
assertEq(threw, 'RangeError', 'status below 200');
try { new Response('x', { status: 600 }); } catch (e) { threw = e.name; }
assertEq(threw, 'RangeError', 'status above 599');
try { new Response('x', { status: 204 }); } catch (e) { threw = e.name; }
assertEq(threw, 'TypeError', 'null body status rejects a body');
return 'ok';
"#);
}

#[test]
fn response_statics_build_special_responses() {
    run(r#"
const j = Response.json({ a: 1 }, { status: 202 });
assertEq(j.status, 202, 'json status');
assertEq(j.headers.get('content-type'), 'application/json', 'json content type');
assertEq(j.ok, true, 'json ok');
let threw = '';
try { Response.json(undefined); } catch (e) { threw = e.name; }
assertEq(threw, 'TypeError', 'unserializable json data');
const red = Response.redirect('https://example.com/next');
assertEq(red.status, 302, 'default redirect status');
assertEq(red.headers.get('location'), 'https://example.com/next', 'location header');
assertEq(red.body, null, 'redirect has no body');
try { Response.redirect('https://x', 300); } catch (e) { threw = e.name; }
assertEq(threw, 'RangeError', 'invalid redirect status');
const err = Response.error();
assertEq(err.status, 0, 'error status');
assertEq(err.ok, false, 'error is not ok');
assertEq(err.body, null, 'error has no body');
return 'ok';
"#);
}

#[test]
fn unsupported_body_init_types_throw() {
    run(r#"
for (const bad of [{}, new Headers(), 42]) {
    let threw = '';
    try { new Response(bad); } catch (e) { threw = e.name; }
    assertEq(threw, 'TypeError', 'unsupported BodyInit throws TypeError');
}
return 'ok';
"#);
}

#[test]
fn body_read_methods_consume_the_host_buffer() {
    let (handle, body) = run_async(
        r#"
const r = new Response('hello world');
const text = await r.text();
assertEq(text, 'hello world', 'text()');
assertEq(r.bodyUsed, true, 'bodyUsed after text');
let rejected = '';
try { await r.text(); } catch (e) { rejected = e.name; }
assertEq(rejected, 'TypeError', 'second read rejects');
const j = Response.json({ a: 1 });
assertEq(JSON.stringify(await j.json()), '{"a":1}', 'json()');
const bad = new Response('not json');
let parseFailed = false;
try { await bad.json(); } catch (e) { parseFailed = true; }
assert(parseFailed, 'invalid json rejects');
const buf = new Response(new Uint8Array([65, 66]));
const ab = await buf.arrayBuffer();
assert(ab instanceof ArrayBuffer, 'arrayBuffer()');
assertEq(JSON.stringify([...new Uint8Array(ab)]), '[65,66]', 'bytes round trip');
const empty = new Response();
assertEq(await empty.text(), '', 'absent body reads as empty');
assertEq(empty.bodyUsed, false, 'absent body is never used');
return 'reads:' + text;
"#,
    );
    assert_eq!(body, "reads:hello world");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn clone_tees_the_body_before_either_side_reads() {
    let (handle, body) = run_async(
        r#"
const r = new Response('payload', { headers: { 'X-A': '1' } });
const c = r.clone();
assertEq(c.status, 200, 'clone keeps status');
assertEq(c.headers.get('x-a'), '1', 'clone copies headers');
assertEq(await r.text(), 'payload', 'original reads');
assertEq(await c.text(), 'payload', 'clone reads');
const used = new Response('x');
await used.text();
let threw = '';
try { used.clone(); } catch (e) { threw = e.name; }
assertEq(threw, 'TypeError', 'cloning a used body throws');
const req = new Request('https://x', { method: 'POST', body: 'rq' });
const reqClone = req.clone();
assertEq(await req.text(), 'rq', 'request original reads');
assertEq(await reqClone.text(), 'rq', 'request clone reads');
return 'cloned';
"#,
    );
    assert_eq!(body, "cloned");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn body_init_sets_content_type_and_url_search_params_serialize() {
    let (handle, body) = run_async(
        r#"
const s = new Response('plain');
assertEq(s.headers.get('content-type'), 'text/plain;charset=UTF-8', 'string content type');
const p = new Response(new URLSearchParams({ a: '1', b: '2' }));
assertEq(p.headers.get('content-type'), 'application/x-www-form-urlencoded;charset=UTF-8', 'params content type');
assertEq(await p.text(), 'a=1&b=2', 'params serialize');
const ab = new Response(new Uint8Array([1]).buffer);
assertEq(ab.headers.get('content-type'), null, 'arraybuffer sets no content type');
const keep = new Response('x', { headers: { 'Content-Type': 'text/custom' } });
assertEq(keep.headers.get('content-type'), 'text/custom', 'explicit content type wins');
const req = new Request('https://x', { method: 'POST', body: new Uint8Array([104, 105]) });
assertEq(await req.text(), 'hi', 'request body read');
return 'typed';
"#,
    );
    assert_eq!(body, "typed");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn fetch_may_return_a_response_instance() {
    let source = format!(
        "{ASSERTS}\nexport default {{ fetch() {{ return new Response('created', {{ status: 201 }}); }} }};"
    );
    let code = compile("plugin.mjs", &source).unwrap().code;
    let out = Worker::new()
        .start_request()
        .unwrap()
        .invoke(
            &code,
            Invocation {
                url: "/".into(),
                method: "GET".into(),
            },
        )
        .unwrap();
    assert_eq!(out, "created");
}

// --- timers / queueMicrotask (spec 01 line 32) ---

#[test]
fn queue_microtask_is_the_engine_builtin() {
    let (handle, body) = run_async(
        r#"
assertEq(typeof queueMicrotask, 'function', 'queueMicrotask exists');
const order = [];
queueMicrotask(() => order.push('micro'));
order.push('sync');
await null;
assertEq(JSON.stringify(order), '["sync","micro"]', 'microtask runs before the await continuation');
return 'micro';
"#,
    );
    assert_eq!(body, "micro");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn interval_fires_while_the_request_is_alive_and_clear_stops_it() {
    let (handle, body) = run_async(
        r#"
assertEq(typeof setInterval, 'function', 'setInterval exists');
assertEq(typeof clearInterval, 'function', 'clearInterval exists');
let ticks = 0;
const id = setInterval(() => { ticks++; }, 20);
await new Promise(r => setTimeout(r, 90));
clearInterval(id);
const afterClear = ticks;
await new Promise(r => setTimeout(r, 60));
assertEq(ticks, afterClear, 'clearInterval stops the timer');
assert(ticks >= 2, 'interval fired repeatedly, got ' + ticks);
return 'ticks';
"#,
    );
    assert_eq!(body, "ticks");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn interval_can_cancel_itself_inside_the_callback() {
    let (handle, body) = run_async(
        r#"
let ticks = 0;
const id = setInterval(() => { ticks++; clearInterval(id); }, 10);
await new Promise(r => setTimeout(r, 80));
assertEq(ticks, 1, 'self-cancelling interval fired exactly once');
return 'self-clear';
"#,
    );
    assert_eq!(body, "self-clear");
    assert_eq!(handle.phase(), Phase::Closed);
}

// --- structuredClone (spec 01 line 33) ---

#[test]
fn structured_clone_copies_plain_data_and_cycles() {
    run(r#"
const obj = { a: 1, nested: { b: [1, 2, { c: 'x' }] } };
const copy = structuredClone(obj);
assertEq(JSON.stringify(copy), JSON.stringify(obj), 'deep copy');
copy.nested.b[0] = 99;
assertEq(obj.nested.b[0], 1, 'independent');
const cyc = { name: 'cyc' };
cyc.self = cyc;
const cycCopy = structuredClone(cyc);
assert(cycCopy !== cyc && cycCopy.self === cycCopy, 'cycle preserved');
const shared = { v: 1 };
const twice = structuredClone({ x: shared, y: shared });
assert(twice.x === twice.y, 'shared references stay shared');
assertEq(structuredClone(undefined), undefined, 'undefined passes through');
assertEq(structuredClone(42n), 42n, 'bigint passes through');
return 'ok';
"#);
}

#[test]
fn structured_clone_handles_builtins() {
    run(r#"
const m = new Map([['k', { n: 1 }]]);
const s = new Set([1, 2]);
const d = new Date(1700000000000);
const re = /ab+c/gi;
const buf = new Uint8Array([1, 2, 3]).buffer;
const u8 = new Uint8Array([4, 5]);
const dv = new DataView(new Uint8Array([9, 8]).buffer);
const err = new TypeError('boom');
const c = structuredClone({ m, s, d, re, buf, u8, dv, err });
assert(c.m instanceof Map && c.m.get('k').n === 1 && c.m.get('k') !== m.get('k'), 'Map deep');
assert(c.s instanceof Set && c.s.has(2), 'Set');
assert(c.d instanceof Date && c.d.getTime() === 1700000000000, 'Date');
assert(c.re instanceof RegExp && c.re.source === 'ab+c' && c.re.flags === 'gi', 'RegExp');
assert(c.buf instanceof ArrayBuffer && new Uint8Array(c.buf)[1] === 2, 'ArrayBuffer');
assert(c.u8 instanceof Uint8Array && c.u8[0] === 4 && c.u8 !== u8, 'typed array');
assert(c.dv instanceof DataView && c.dv.getUint8(0) === 9, 'DataView');
assert(c.err instanceof Error && c.err.message === 'boom' && c.err.name === 'TypeError', 'Error');
return 'ok';
"#);
}

#[test]
fn structured_clone_rejects_uncloneable_values_and_transfer() {
    run(r#"
function name(fn) { try { fn(); return ''; } catch (e) { return e.name; } }
assertEq(name(() => structuredClone(() => 1)), 'DataCloneError', 'function');
class Custom { constructor() { this.x = 1; } }
assertEq(name(() => structuredClone(new Custom())), 'DataCloneError', 'custom prototype');
assertEq(name(() => structuredClone({}, { transfer: [new ArrayBuffer(0)] })), 'DataCloneError', 'non-empty transfer');
structuredClone({}, { transfer: [] });
return 'ok';
"#);
}

// --- atob / btoa (spec 01 line 34) ---

#[test]
fn atob_and_btoa_round_trip_latin1() {
    run(r#"
assertEq(btoa('hello'), 'aGVsbG8=', 'btoa');
assertEq(atob('aGVsbG8='), 'hello', 'atob');
assertEq(atob(btoa('\x00\xff')), '\x00\xff', 'latin-1 bytes round trip');
assertEq(btoa(''), '', 'empty');
assertEq(atob(''), '', 'empty decode');
assertEq(atob(' aGVs\nbG8= '), 'hello', 'ascii whitespace tolerated');
assertEq(atob('aGVsbG8'), 'hello', 'forgiving decode without padding');
return 'ok';
"#);
}

#[test]
fn atob_btoa_reject_invalid_input() {
    run(r#"
function name(fn) { try { fn(); return ''; } catch (e) { return e.name; } }
assertEq(name(() => btoa('中')), 'InvalidCharacterError', 'btoa rejects code points above 0xff');
assertEq(name(() => atob('a!b=')), 'InvalidCharacterError', 'atob rejects illegal chars');
assertEq(name(() => atob('a')), 'InvalidCharacterError', 'length % 4 == 1');
assertEq(name(() => atob('ab==cd')), 'InvalidCharacterError', 'padding in the middle');
return 'ok';
"#);
}

// --- crypto (spec 01 line 35) ---

#[test]
fn get_random_values_fills_integer_arrays_in_place() {
    run(r#"
const u8 = new Uint8Array(16);
const ret = crypto.getRandomValues(u8);
assert(ret === u8, 'returns the same array');
assert(u8.some(b => b !== 0), 'filled with random bytes');
crypto.getRandomValues(new Int32Array(4));
crypto.getRandomValues(new Uint8Array(0));
return 'ok';
"#);
}

#[test]
fn get_random_values_validates_type_and_length() {
    run(r#"
function name(fn) { try { fn(); return ''; } catch (e) { return e.name; } }
assertEq(name(() => crypto.getRandomValues(new Float64Array(2))), 'TypeError', 'float arrays rejected');
assertEq(name(() => crypto.getRandomValues(new DataView(new ArrayBuffer(4)))), 'TypeError', 'DataView rejected');
assertEq(name(() => crypto.getRandomValues([1, 2])), 'TypeError', 'plain arrays rejected');
assertEq(name(() => crypto.getRandomValues(new Uint8Array(65537))), 'QuotaExceededError', 'over 64 KiB');
crypto.getRandomValues(new Uint8Array(65536));
return 'ok';
"#);
}

#[test]
fn random_uuid_is_version_4() {
    run(r#"
const id = crypto.randomUUID();
assertEq(id.length, 36, 'uuid length');
assertEq(id[14], '4', 'version 4');
assert('89ab'.includes(id[19]), 'variant');
assert(/^[0-9a-f-]+$/.test(id), 'hex format');
assert(crypto.randomUUID() !== id, 'unique');
return 'ok';
"#);
}

#[test]
fn subtle_digest_matches_known_vectors() {
    let (handle, body) = run_async(
        r#"
async function hex(p) { return [...new Uint8Array(await p)].map(b => b.toString(16).padStart(2, '0')).join(''); }
const d256 = await hex(crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc')));
assertEq(d256, 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad', 'SHA-256 of abc');
const d384 = await hex(crypto.subtle.digest({ name: 'SHA-384' }, new TextEncoder().encode('abc')));
assertEq(d384.slice(0, 8), 'cb00753f', 'SHA-384 prefix of abc');
const d512 = await hex(crypto.subtle.digest('SHA-512', new Uint8Array([97, 98, 99]).buffer));
assertEq(d512.slice(0, 8), 'ddaf35a1', 'SHA-512 prefix of abc');
let rejected = '';
try { await crypto.subtle.digest('SHA-1', new Uint8Array(1)); } catch (e) { rejected = e.name; }
assertEq(rejected, 'NotSupportedError', 'SHA-1 rejected');
let threw = '';
try { await crypto.subtle.digest('SHA-256', 42); } catch (e) { threw = e.name; }
assertEq(threw, 'TypeError', 'non-buffer data rejected');
return 'digest ok';
"#,
    );
    assert_eq!(body, "digest ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn subtle_hmac_sign_and_verify_match_rfc4231() {
    let (handle, body) = run_async(
        r#"
const key = await crypto.subtle.importKey('raw', new Uint8Array(20).fill(0x0b), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign', 'verify']);
const sig = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode('Hi There'));
const hex = [...new Uint8Array(sig)].map(b => b.toString(16).padStart(2, '0')).join('');
assertEq(hex, 'b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7', 'RFC 4231 test case 1');
assertEq(await crypto.subtle.verify('HMAC', key, sig, new TextEncoder().encode('Hi There')), true, 'verify good');
assertEq(await crypto.subtle.verify('HMAC', key, sig, new TextEncoder().encode('Hi There!')), false, 'verify bad data');
assertEq(await crypto.subtle.verify('HMAC', key, sig.slice(0, 10), new TextEncoder().encode('Hi There')), false, 'short signature is false');
const verifyOnly = await crypto.subtle.importKey('raw', new Uint8Array([1]), { name: 'HMAC', hash: { name: 'SHA-512' } }, true, ['verify']);
let rejected = '';
try { await crypto.subtle.sign('HMAC', verifyOnly, new Uint8Array(1)); } catch (e) { rejected = e.name; }
assertEq(rejected, 'InvalidAccessError', 'sign without the sign usage');
const sig256 = await crypto.subtle.sign({ name: 'HMAC' }, key, new Uint8Array([1, 2]));
assertEq(sig256.byteLength, 32, 'SHA-256 signature length');
return 'hmac ok';
"#,
    );
    assert_eq!(body, "hmac ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn import_key_validates_format_algorithm_and_usages() {
    let (handle, body) = run_async(
        r#"
async function name(p) { try { await p; return ''; } catch (e) { return e.name; } }
const raw = new Uint8Array([1, 2, 3]);
assertEq(await name(crypto.subtle.importKey('jwk', raw, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign'])), 'NotSupportedError', 'only raw');
assertEq(await name(crypto.subtle.importKey('raw', raw, { name: 'AES-GCM' }, false, ['sign'])), 'NotSupportedError', 'only HMAC');
assertEq(await name(crypto.subtle.importKey('raw', raw, { name: 'HMAC', hash: 'SHA-1' }, false, ['sign'])), 'NotSupportedError', 'hash subset');
assertEq(await name(crypto.subtle.importKey('raw', raw, { name: 'HMAC', hash: 'SHA-256' }, false, ['encrypt'])), 'SyntaxError', 'bad usage');
assertEq(await name(crypto.subtle.importKey('raw', raw, { name: 'HMAC', hash: 'SHA-256' }, false, [])), 'SyntaxError', 'empty usages');
assertEq(await name(crypto.subtle.importKey('raw', new Uint8Array(0), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign'])), 'DataError', 'empty key');
const key = await crypto.subtle.importKey('raw', raw, { name: 'HMAC', hash: 'SHA-384' }, false, ['sign']);
assert(key instanceof CryptoKey, 'CryptoKey instance');
let ctorThrew = '';
try { new CryptoKey(); } catch (e) { ctorThrew = e.name; }
assertEq(ctorThrew, 'TypeError', 'CryptoKey is not constructible');
return 'import ok';
"#,
    );
    assert_eq!(body, "import ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

// --- console (spec 01 line 36) ---

#[test]
fn console_methods_format_and_reach_the_host_sink() {
    gateway::web::console::start_capture();
    let body = run(r#"
console.log('hello', 42, { a: 1 });
console.info('info-message');
console.warn('warn-message');
console.error('error-message', null, undefined);
console.debug('debug-message');
const cyc = {}; cyc.self = cyc;
console.log(cyc);
return 'ok';
"#);
    let logs = gateway::web::console::drain_capture();
    gateway::web::console::stop_capture();
    assert_eq!(body, "ok");
    let find = |level: &str| {
        logs.iter()
            .find(|r| r.level == level)
            .map(|r| r.message.clone())
    };
    assert_eq!(find("log").as_deref(), Some("hello 42 {\"a\":1}"));
    assert_eq!(find("info").as_deref(), Some("info-message"));
    assert_eq!(find("warn").as_deref(), Some("warn-message"));
    assert_eq!(
        find("error").as_deref(),
        Some("error-message null undefined")
    );
    assert_eq!(find("debug").as_deref(), Some("debug-message"));
    // Cycles do not kill the request; the record holds owned text only.
    assert!(logs.iter().any(|r| r.message == "[unserializable]"));
}

// --- M4 review follow-ups: structuredClone fidelity ---

#[test]
fn structured_clone_preserves_shared_identity_for_wrappers() {
    run(r#"
const d = new Date(1726761600000);
const re = /ab+c/gi;
const err = new TypeError('boom');
const copy = structuredClone({ a: d, b: d, c: re, d: re, e: err, f: err });
assert(copy.a === copy.b, 'shared Date stays shared');
assert(copy.c === copy.d, 'shared RegExp stays shared');
assert(copy.e === copy.f, 'shared Error stays shared');
assert(copy.a !== d, 'Date is cloned, not aliased');
assertEq(copy.a.getTime(), 1726761600000, 'Date payload survives');
assertEq(copy.c.source, 'ab+c', 'RegExp payload survives');
assertEq(copy.e.name, 'TypeError', 'Error name survives');
return 'ok';
"#);
}

#[test]
fn structured_clone_rejects_non_object_options() {
    run(r#"
for (const bad of [42, 'transfer', true]) {
    let name = '';
    try { structuredClone({}, bad); } catch (e) { name = e.name; }
    assertEq(name, 'TypeError', 'non-object options is a Web IDL TypeError');
}
return 'ok';
"#);
}

#[test]
fn request_clone_rejects_a_consumed_body() {
    let (handle, body) = run_async(
        r#"
const req = new Request('https://x', { method: 'POST', body: 'rq' });
await req.text();
let name = '';
try { req.clone(); } catch (e) { name = e.name; }
assertEq(name, 'TypeError', 'cloning a consumed Request body throws');
return 'ok';
"#,
    );
    assert_eq!(body, "ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

// --- Hono acceptance: the real framework on the closed API list ---

/// Vendored esbuild bundle of hono 4.13.8 (`hono/quick` plus the bearerAuth,
/// cors and logger middleware); see `tests/fixtures/README.md` for
/// provenance and the pinned sha256. Tests are fully offline.
const HONO_BUNDLE: &str = include_str!("fixtures/hono-quick-4.13.8.mjs");

/// A Hono app covering the acceptance paths: routing, bearer authentication,
/// cors, logging and handler exceptions. `console.log` is patched before the
/// middleware reads it, so log lines are observable per request engine.
const HONO_APP: &str = r#"
const { Hono, bearerAuth, cors, logger } = globalThis.HonoQuick;
const logLines = [];
const hostLog = console.log;
console.log = (...args) => { logLines.push(args.join(' ')); hostLog(...args); };
const app = new Hono();
app.use(logger());
app.use('/secure/*', bearerAuth({ token: 's3cret' }));
app.use('/api/*', cors());
app.get('/hello/:name', (c) => c.text('hello ' + c.req.param('name')));
app.get('/secure/data', (c) => c.text('classified'));
app.get('/api/info', (c) => c.json({ ok: true }));
app.get('/boom', () => { throw new Error('plugin boom'); });
app.get('/boom-object', () => { throw { code: 'plugin boom' }; });
app.get('/secure-probe', async (c) => {
    const noToken = await app.fetch(new Request('http://test.local/secure/data'));
    const wrong = await app.fetch(new Request('http://test.local/secure/data', { headers: { authorization: 'Bearer wrong' } }));
    const right = await app.fetch(new Request('http://test.local/secure/data', { headers: { authorization: 'Bearer s3cret' } }));
    return c.json({ noToken: noToken.status, wrong: wrong.status, right: right.status, www: noToken.headers.get('www-authenticate'), rightBody: await right.text() });
});
app.get('/cors-probe', async (c) => {
    const preflight = await app.fetch(new Request('http://test.local/api/info', { method: 'OPTIONS', headers: { origin: 'https://a.example', 'access-control-request-method': 'GET' } }));
    const simple = await app.fetch(new Request('http://test.local/api/info', { headers: { origin: 'https://a.example' } }));
    return c.json({ preflightStatus: preflight.status, preflightAllow: preflight.headers.get('access-control-allow-origin'), simpleAllow: simple.headers.get('access-control-allow-origin'), simpleOk: (await simple.json()).ok });
});
app.get('/logs-probe/:name', async (c) => {
    await app.fetch(new Request('http://test.local/hello/' + c.req.param('name')));
    return c.json(logLines);
});
export default app;
"#;

/// Run a Hono app against the M2 async request path and return the delivered
/// status, headers and body.
fn run_hono(app: &str, url: &str, method: &str) -> (u16, Vec<(String, String)>, String) {
    let source = format!("{HONO_BUNDLE}\n{app}");
    let code = compile("plugin.mjs", &source).unwrap().code;
    let worker = Worker::new();
    let mut exec = Executor::new();
    let engine = worker.start_request().unwrap();
    let (handle, task) = engine.into_task(
        &exec.handle(),
        &code,
        Invocation {
            url: url.into(),
            method: method.into(),
        },
    );
    exec.spawn(task.drive());
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            // Stand-in for the 02 HTTP layer.
            let deadline = Instant::now() + Duration::from_secs(5);
            while h.phase() != Phase::Responding {
                assert!(Instant::now() < deadline, "never reached Responding");
                std::thread::sleep(Duration::from_millis(1));
            }
            let status = h.response_status().expect("status visible once Responding");
            let headers = h
                .response_headers()
                .expect("headers visible once Responding");
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            (status, headers, body)
        })
    };
    exec.run();
    let delivered = http.join().unwrap();
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
    delivered
}

#[test]
fn hono_quick_routes_a_parameterized_path() {
    let (status, headers, body) = run_hono(HONO_APP, "http://test.local/hello/world", "GET");
    assert_eq!(status, 200);
    assert_eq!(body, "hello world");
    // The host default for a string body (hono's fast path is
    // `new Response(text)`, which picks up the runtime default).
    assert!(headers.contains(&(
        "content-type".to_string(),
        "text/plain;charset=UTF-8".to_string()
    )));
}

#[test]
fn hono_bearer_auth_rejects_the_incoming_request_without_a_token() {
    let (status, headers, body) = run_hono(HONO_APP, "http://test.local/secure/data", "GET");
    assert_eq!(status, 401);
    assert_eq!(body, "Unauthorized");
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "www-authenticate" && v.starts_with("Bearer")),
        "WWW-Authenticate header present: {headers:?}"
    );
}

#[test]
fn hono_bearer_auth_rejects_wrong_and_accepts_right_tokens() {
    let (status, _, body) = run_hono(HONO_APP, "http://test.local/secure-probe", "GET");
    assert_eq!(status, 200);
    assert_eq!(
        body,
        r#"{"noToken":401,"wrong":401,"right":200,"www":"Bearer realm=\"\"","rightBody":"classified"}"#
    );
}

#[test]
fn hono_cors_sets_headers_and_answers_preflight() {
    let (status, _, body) = run_hono(HONO_APP, "http://test.local/cors-probe", "GET");
    assert_eq!(status, 200);
    assert_eq!(
        body,
        r#"{"preflightStatus":204,"preflightAllow":"*","simpleAllow":"*","simpleOk":true}"#
    );
    // The incoming request path gets the cors header too.
    let (status, headers, _) = run_hono(HONO_APP, "http://test.local/api/info", "GET");
    assert_eq!(status, 200);
    assert!(headers.contains(&("access-control-allow-origin".to_string(), "*".to_string())));
}

#[test]
fn hono_logger_writes_request_lines_to_the_console() {
    let (status, _, body) = run_hono(HONO_APP, "http://test.local/logs-probe/hono", "GET");
    assert_eq!(status, 200);
    assert!(
        body.contains("<-- GET /hello/hono"),
        "arrival line logged: {body}"
    );
    assert!(
        body.contains("--> GET /hello/hono") && body.contains("200"),
        "completion line logged with status: {body}"
    );
}

#[test]
fn hono_handler_exception_is_answered_500_by_the_framework() {
    let (status, _, body) = run_hono(HONO_APP, "http://test.local/boom", "GET");
    assert_eq!(status, 500);
    assert_eq!(body, "Internal Server Error");
}

#[test]
fn hono_unhandled_exception_enters_request_error_handling() {
    // A non-Error throw escapes Hono's error handler, rejects the entry
    // promise and lands in the request error path.
    let source = format!("{HONO_BUNDLE}\n{HONO_APP}");
    let code = compile("plugin.mjs", &source).unwrap().code;
    let worker = Worker::new();
    let mut exec = Executor::new();
    let engine = worker.start_request().unwrap();
    let (handle, task) = engine.into_task(
        &exec.handle(),
        &code,
        Invocation {
            url: "http://test.local/boom-object".into(),
            method: "GET".into(),
        },
    );
    exec.spawn(task.drive());
    exec.run();
    assert!(handle.outcome().unwrap().is_err());
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

// --- workerd ported assertions (M4 Task 3) ---

/// The node:assert subset shim prepended to every workerd fixture.
const WORKERD_ASSERT_SHIM: &str = include_str!("fixtures/workerd/node-assert-shim.mjs");

/// Compile a workerd fixture (`tests/fixtures/workerd/*.js`) together with the
/// node:assert shim and drive it down the M2 async request path. A failed
/// assertion inside the plugin throws in JS, rejects the entry promise and
/// fails the host test with the JS error text. Returns the delivered body.
fn run_workerd(fixture: &str) -> String {
    let source = format!("{WORKERD_ASSERT_SHIM}\n{fixture}");
    let code = compile("plugin.mjs", &source).unwrap().code;
    let worker = Worker::new();
    let mut exec = Executor::new();
    let engine = worker.start_request().unwrap();
    let (handle, task) = engine.into_task(
        &exec.handle(),
        &code,
        Invocation {
            url: "/".into(),
            method: "GET".into(),
        },
    );
    exec.spawn(task.drive());
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            // Stand-in for the 02 HTTP layer.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match h.phase() {
                    Phase::Responding => {
                        let body = h.take_response().expect("response visible once Responding");
                        h.finish_response();
                        return body;
                    }
                    Phase::Closed => return String::new(),
                    _ => {
                        assert!(Instant::now() < deadline, "never reached Responding");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        })
    };
    exec.run();
    let body = http.join().unwrap();
    // A rejected entry promise means a fixture assertion failed: surface the
    // JS error instead of the empty body.
    if let Some(outcome) = handle.outcome() {
        outcome.unwrap();
    }
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
    body
}

#[test]
fn workerd_url() {
    let body = run_workerd(include_str!("fixtures/workerd/url.js"));
    assert_eq!(body, "workerd-url ok");
}

#[test]
fn workerd_encoding() {
    let body = run_workerd(include_str!("fixtures/workerd/encoding.js"));
    assert_eq!(body, "workerd-encoding ok");
}

#[test]
fn workerd_abortsignal() {
    let body = run_workerd(include_str!("fixtures/workerd/abortsignal.js"));
    assert_eq!(body, "workerd-abortsignal ok");
}

#[test]
fn workerd_http() {
    let body = run_workerd(include_str!("fixtures/workerd/http.js"));
    assert_eq!(body, "workerd-http ok");
}

// --- M4 framework enablement: body handles and the iterable protocol ---

#[test]
fn body_handle_round_trips_as_body_init() {
    let (handle, body) = run_async(
        r#"
const r = new Response('payload', { headers: { 'X-A': '1' } });
const h = r.body;
assert(h !== null && typeof h === 'object', 'body handle is an opaque object');
assertEq(Object.getOwnPropertyNames(h).length, 0, 'handle has no visible properties');
// A handle passed back as BodyInit keeps the body host-side (Hono's
// c.newResponse(res.body, res) relies on this).
const wrapped = new Response(h, { status: 201, headers: r.headers });
assertEq(wrapped.status, 201, 'status from init');
assertEq(await wrapped.text(), 'payload', 'body survives the round trip');
assertEq(r.bodyUsed, true, 'the shared body is consumed exactly once');
return 'ok';
"#,
    );
    assert_eq!(body, "ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn body_handle_rejects_a_consumed_body() {
    let (handle, body) = run_async(
        r#"
const r = new Response('payload');
const h = r.body;
await r.text();
// Stub semantics until 02 streams: once consumed, the getter reports null.
assertEq(r.body, null, 'consumed body getter is null');
let name = '';
try { new Response(h); } catch (e) { name = e.name; }
assertEq(name, 'TypeError', 'a consumed body handle is rejected');
return 'ok';
"#,
    );
    assert_eq!(body, "ok");
    assert_eq!(handle.phase(), Phase::Closed);
}

#[test]
fn headers_and_search_params_follow_the_iterable_protocol() {
    run(r#"
const h = new Headers([['b', '2'], ['a', '1']]);
assertEq(typeof h.entries().next, 'function', 'entries() returns an iterator');
assertEq(h[Symbol.iterator], h.entries, 'Symbol.iterator aliases entries');
const pairs = [];
for (const [k, v] of h) pairs.push(k + '=' + v);
assertEq(JSON.stringify(pairs), '["a=1","b=2"]', 'for..of over Headers');
const it = h.entries();
const first = it.next();
assertEq(JSON.stringify(first.value), '["a","1"]', 'iterator next()');
assertEq(first.done, false, 'not done');
const p = new URLSearchParams('b=2&a=1');
assertEq(typeof p.entries().next, 'function', 'params entries() iterator');
assertEq(p[Symbol.iterator], p.entries, 'params Symbol.iterator aliases entries');
const seen = [];
for (const [k, v] of p) seen.push(k + '=' + v);
assertEq(JSON.stringify(seen), '["b=2","a=1"]', 'for..of over URLSearchParams keeps insertion order');
return 'ok';
"#);
}
