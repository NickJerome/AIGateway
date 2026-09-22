// Versioned data graph, deliberately independent of QuickJS bytecode layouts.
(() => {
  const isProxy = __isProxy;
  const fail = (m) => {
    const e = new Error(m);
    e.name = "DataCloneError";
    throw e;
  };
  const types = {
    Int8Array,
    Uint8Array,
    Uint8ClampedArray,
    Int16Array,
    Uint16Array,
    Int32Array,
    Uint32Array,
    Float32Array,
    Float64Array,
    BigInt64Array,
    BigUint64Array,
    DataView,
  };
  globalThis.__cloneEncode = (value) => {
    const nodes = [],
      seen = new Map();
    function ref(v) {
      if (v === undefined) return ["u"];
      if (typeof v === "bigint") return ["i", String(v)];
      if (typeof v === "number" && (!Number.isFinite(v) || Object.is(v, -0)))
        return ["n", Object.is(v, -0) ? "-0" : String(v)];
      if (typeof v === "symbol" || typeof v === "function")
        return fail("unsupported clone value");
      if (v === null || typeof v !== "object") return ["v", v];
      if (isProxy(v)) return fail("Proxy cannot be cloned");
      if (seen.has(v)) return ["r", seen.get(v)];
      const id = nodes.length;
      seen.set(v, id);
      nodes.push(null);
      let node;
      if (Array.isArray(v))
        node = ["array", v.length, Object.keys(v).map((k) => [k, ref(v[k])])];
      else if (v instanceof Map)
        node = ["map", [...v].map(([k, x]) => [ref(k), ref(x)])];
      else if (v instanceof Set) node = ["set", [...v].map(ref)];
      else if (v instanceof Date) node = ["date", ref(v.getTime())];
      else if (v instanceof RegExp)
        node = ["regexp", v.source, v.flags, v.lastIndex];
      else if (v instanceof ArrayBuffer)
        node = ["buffer", Array.from(new Uint8Array(v))];
      else if (ArrayBuffer.isView(v)) {
        const proto =
          v instanceof DataView
            ? DataView.prototype
            : Object.getPrototypeOf(Uint8Array.prototype);
        const get = (k) =>
          Object.getOwnPropertyDescriptor(proto, k).get.call(v);
        const type = Object.entries(types).find(
          ([, C]) => Object.getPrototypeOf(v) === C.prototype,
        )?.[0];
        if (!type) return fail("unsupported view");
        node = [
          "view",
          type,
          ref(get("buffer")),
          get("byteOffset"),
          v instanceof DataView ? get("byteLength") : get("length"),
        ];
      } else if (v instanceof Error)
        node = ["error", String(v.name), String(v.message), ref(v.cause)];
      else if (
        Object.getPrototypeOf(v) === Object.prototype ||
        Object.getPrototypeOf(v) === null
      )
        node = ["object", Object.keys(v).map((k) => [k, ref(v[k])])];
      else return fail("custom instances cannot be cloned");
      nodes[id] = node;
      return ["r", id];
    }
    const root = ref(value);
    return JSON.stringify({ version: 1, root, nodes });
  };
  globalThis.__cloneDecode = (text) => {
    const data = JSON.parse(text);
    if (data.version !== 1 || !Array.isArray(data.nodes))
      return fail("unknown clone encoding");
    const out = new Map();
    const set = (o, k, v) =>
      Object.defineProperty(o, k, {
        value: v,
        writable: true,
        enumerable: true,
        configurable: true,
      });
    function read(r) {
      if (r[0] === "u") return undefined;
      if (r[0] === "v") return r[1];
      if (r[0] === "i") return BigInt(r[1]);
      if (r[0] === "n") return r[1] === "-0" ? -0 : Number(r[1]);
      if (r[0] !== "r" || !Number.isInteger(r[1]) || !data.nodes[r[1]])
        return fail("invalid clone reference");
      const id = r[1];
      if (out.has(id)) return out.get(id);
      const n = data.nodes[id];
      let v;
      switch (n[0]) {
        case "array":
          v = new Array(n[1]);
          break;
        case "object":
          v = {};
          break;
        case "map":
          v = new Map();
          break;
        case "set":
          v = new Set();
          break;
        case "date":
          v = new Date(read(n[1]));
          break;
        case "regexp":
          v = new RegExp(n[1], n[2]);
          v.lastIndex = n[3];
          break;
        case "buffer":
          v = new Uint8Array(n[1]).buffer;
          break;
        case "view":
          if (!Object.hasOwn(types, n[1])) return fail("invalid view");
          v = new types[n[1]](read(n[2]), n[3], n[4]);
          break;
        case "error":
          v = new Error(n[2]);
          v.name = n[1];
          break;
        default:
          return fail("invalid clone node");
      }
      out.set(id, v);
      if (n[0] === "array") for (const [k, x] of n[2]) set(v, k, read(x));
      if (n[0] === "object") for (const [k, x] of n[1]) set(v, k, read(x));
      if (n[0] === "map") for (const [k, x] of n[1]) v.set(read(k), read(x));
      if (n[0] === "set") for (const x of n[1]) v.add(read(x));
      if (n[0] === "error") v.cause = read(n[3]);
      return v;
    }
    return read(data.root);
  };
})();
