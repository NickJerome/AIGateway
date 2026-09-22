//! `structuredClone` (spec 01 line 33): plain objects, circular references,
//! arrays, Map/Set, Date/RegExp, ArrayBuffer/typed arrays/DataView, Error.
//! A non-empty transfer list is an explicit error. Functions and instances
//! with a custom prototype throw a `DataCloneError`-named Error (the engine
//! has no DOMException).
//!
//! The algorithm runs as an installed JS prelude so engine builtins (Map,
//! Set, Date, RegExp, typed arrays) do the heavy lifting; the JS executes
//! under the same per-request slice clock as plugin code. Known deviation:
//! a Proxy is indistinguishable from its target without engine internals,
//! so a proxy of a cloneable value clones as its target.

use rquickjs::{Ctx, Result};

const STRUCTURED_CLONE_PRELUDE: &str = r#"
globalThis.structuredClone = function structuredClone(value, options) {
    function dataCloneError(message) {
        const e = new Error(message);
        e.name = 'DataCloneError';
        return e;
    }
    if (options !== undefined && options !== null) {
        // Web IDL dictionary conversion: a non-object options argument is a
        // TypeError, not a silently ignored value.
        if (typeof options !== 'object' && typeof options !== 'function') {
            throw new TypeError('structuredClone options must be an object');
        }
        const transfer = options.transfer;
        if (transfer !== undefined && transfer !== null) {
            const items = Array.from(transfer);
            if (items.length > 0) {
                throw dataCloneError('structuredClone transfer list is not supported');
            }
        }
    }
    const memo = new Map();
    const isProxy=__isProxy;
    function clone(v) {
        if (isProxy(v)) throw dataCloneError('Proxy cannot be structured-cloned');
        if (typeof v === 'function') {
            throw dataCloneError('function cannot be structured-cloned');
        }
        if (v === null || typeof v !== 'object') {
            return v;
        }
        if (memo.has(v)) {
            return memo.get(v);
        }
        let out;
        if (Array.isArray(v)) {
            out = new Array(v.length);
            memo.set(v, out);
            for (let i = 0; i < v.length; i++) {
                if (i in v) out[i] = clone(v[i]);
            }
            return out;
        }
        if (v instanceof Map) {
            out = new Map();
            memo.set(v, out);
            for (const [k, val] of v) out.set(clone(k), clone(val));
            return out;
        }
        if (v instanceof Set) {
            out = new Set();
            memo.set(v, out);
            for (const item of v) out.add(clone(item));
            return out;
        }
        if (v instanceof Date) {
            out = new Date(v.getTime());
            memo.set(v, out);
            return out;
        }
        if (v instanceof RegExp) {
            out = new RegExp(v.source, v.flags);
            out.lastIndex = v.lastIndex;
            memo.set(v, out);
            return out;
        }
        if (v instanceof ArrayBuffer) {
            out = v.slice(0);
            memo.set(v, out);
            return out;
        }
        if (ArrayBuffer.isView(v)) {
            const clonedBuffer = clone(v.buffer);
            if (v instanceof DataView) {
                out = new DataView(clonedBuffer, v.byteOffset, v.byteLength);
            } else {
                out = new v.constructor(clonedBuffer, v.byteOffset, v.length);
            }
            memo.set(v, out);
            return out;
        }
        if (v instanceof Error) {
            out = new v.constructor(v.message);
            out.name = v.name;
            memo.set(v, out);
            return out;
        }
        const proto = Object.getPrototypeOf(v);
        if (proto === Object.prototype || proto === null) {
            out = {};
            memo.set(v, out);
            for (const key of Object.keys(v)) {
                Object.defineProperty(out, key, {
                    value: clone(v[key]), writable: true,
                    enumerable: true, configurable: true
                });
            }
            return out;
        }
        throw dataCloneError('object with a custom prototype cannot be structured-cloned');
    }
    return clone(value);
};
"#;

/// Install `structuredClone` onto the context globals.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    ctx.globals().set(
        "__isProxy",
        rquickjs::Function::new(ctx.clone(), |value: rquickjs::Value| unsafe {
            rquickjs::qjs::JS_IsProxy(value.as_raw())
        })?,
    )?;
    ctx.eval::<(), _>(STRUCTURED_CLONE_PRELUDE)
}
