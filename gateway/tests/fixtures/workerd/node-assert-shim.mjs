// node:assert subset shim for the ported workerd test fixtures.
//
// Implements just the functions the ported assertions use:
//   strictEqual / deepStrictEqual / ok / throws / match / fail
// Every failure throws an Error carrying a descriptive message, which the
// host test harness surfaces as a request error. Test-only fixture code:
// never referenced from src/.

function fail(message) {
  throw new Error(message === undefined ? 'assert.fail() called' : String(message));
}

function formatValue(value) {
  if (value instanceof Uint8Array) return `Uint8Array[${[...value].join(',')}]`;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function ok(value, message) {
  if (!value) {
    throw new Error(message || `assert.ok failed: value is ${formatValue(value)}`);
  }
}

function strictEqual(actual, expected, message) {
  // SameValue: Object.is semantics (NaN equals NaN, -0 differs from +0).
  if (!Object.is(actual, expected)) {
    const detail = `expected ${formatValue(expected)}, got ${formatValue(actual)}`;
    throw new Error(message ? `${message}: ${detail}` : `assert.strictEqual failed: ${detail}`);
  }
}

function isPlainObject(value) {
  if (value === null || typeof value !== 'object') return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

function deepStrictEqual(actual, expected, message) {
  if (!deepEquals(actual, expected)) {
    const detail = `expected ${formatValue(expected)}, got ${formatValue(actual)}`;
    throw new Error(message ? `${message}: ${detail}` : `assert.deepStrictEqual failed: ${detail}`);
  }
}

function deepEquals(a, b) {
  if (Object.is(a, b)) return true;
  if (typeof a !== typeof b || a === null || b === null) return false;
  if (typeof a !== 'object') return false;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, i) => deepEquals(item, b[i]));
  }
  if (a instanceof Uint8Array || b instanceof Uint8Array) {
    if (!(a instanceof Uint8Array) || !(b instanceof Uint8Array) || a.length !== b.length) {
      return false;
    }
    return a.every((byte, i) => byte === b[i]);
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    return keysA.every((key) => Object.hasOwn(b, key) && deepEquals(a[key], b[key]));
  }
  return false;
}

function match(string, regexp, message) {
  if (!(regexp instanceof RegExp)) {
    throw new Error('assert.match expects a RegExp as the second argument');
  }
  if (typeof string !== 'string' || !regexp.test(string)) {
    throw new Error(
      message || `assert.match failed: ${formatValue(string)} does not match ${regexp}`
    );
  }
}

function throws(fn, expected, message) {
  if (typeof fn !== 'function') {
    throw new Error('assert.throws expects a function as the first argument');
  }
  let caught;
  try {
    fn();
  } catch (err) {
    caught = err;
  }
  if (caught === undefined) {
    throw new Error(message || 'assert.throws failed: function did not throw');
  }
  if (expected === undefined) return caught;
  if (expected instanceof RegExp) {
    const text = caught && caught.message !== undefined ? String(caught.message) : String(caught);
    if (!expected.test(text)) {
      throw new Error(
        message || `assert.throws failed: message ${formatValue(text)} does not match ${expected}`
      );
    }
    return caught;
  }
  if (typeof expected === 'function') {
    if (!(caught instanceof expected)) {
      throw new Error(
        message || `assert.throws failed: ${caught} is not an instance of ${expected.name}`
      );
    }
    return caught;
  }
  if (typeof expected === 'object' && expected !== null) {
    if (expected.name !== undefined) {
      const name = caught && caught.name !== undefined ? caught.name : undefined;
      if (name !== expected.name) {
        throw new Error(
          message || `assert.throws failed: expected error name ${expected.name}, got ${name}`
        );
      }
    }
    if (expected.message !== undefined) {
      const text = caught && caught.message !== undefined ? String(caught.message) : String(caught);
      const okMessage =
        expected.message instanceof RegExp
          ? expected.message.test(text)
          : text === expected.message;
      if (!okMessage) {
        throw new Error(
          message ||
            `assert.throws failed: message ${formatValue(text)} does not match ${expected.message}`
        );
      }
    }
    return caught;
  }
  throw new Error('assert.throws: unsupported expectation argument');
}

// eslint-disable-next-line no-unused-vars
const assert = { strictEqual, deepStrictEqual, ok, throws, match, fail };
