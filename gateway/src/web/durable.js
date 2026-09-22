(() => {
  const encode = __cloneEncode,
    decode = __cloneDecode,
    host = __host;
  const raw = (op, args) => host(op, JSON.stringify(args)).promise;
  const contexts = new WeakMap(),
    stack = [];
  let current = null;
  let watchdog;
  globalThis.__gatewayPromiseHook = (type, p, parent) => {
    if (type === 0) {
      const owner = current ?? contexts.get(parent);
      if (owner) contexts.set(p, owner);
    } else if (type === 1) {
      stack.push(current);
      current = contexts.get(p) ?? null;
    } else if (type === 2) current = stack.pop() ?? null;
  };
  function within(owner, fn) {
    const old = current;
    current = owner;
    try {
      return fn();
    } finally {
      current = old;
    }
  }
  const objects = new Map();
  const allowed = (method) =>
    typeof method === "string" &&
    !method.startsWith("_") &&
    ![
      "then",
      "constructor",
      "toString",
      "valueOf",
      "hasOwnProperty",
      "isPrototypeOf",
      "propertyIsEnumerable",
      "toLocaleString",
    ].includes(method);
  globalThis.__durableBindings = (env, bindings) => {
    for (const [binding, namespace] of Object.entries(bindings)) {
      const ids = new WeakMap();
      env[binding] = Object.freeze({
        idFromName(name) {
          name = String(name);
          if (new TextEncoder().encode(name).length > 4096)
            throw new TypeError("DO name too long");
          const id = Object.freeze({
            toString: () => __objectId(namespace, name),
            name,
          });
          ids.set(id, name);
          return id;
        },
        get(id) {
          if (!ids.has(id))
            throw new TypeError("object ID belongs to another namespace");
          const name = ids.get(id);
          return new Proxy(Object.create(null), {
            get(_, method) {
              if (method === "then") return undefined;
              if (!allowed(method))
                throw new TypeError("method is not callable");
              return async (...args) =>
                decode(
                  await __durableInput(
                    raw("do.call", {
                      binding,
                      name,
                      method,
                      args: encode(args),
                    }),
                  ),
                );
            },
          });
        },
      });
    }
  };
  class Gate {
    tail = Promise.resolve();
    locks = new Map();
    waiters = [];
    writes = new Set();
    error = null;
    init = null;
    initializers = new Map();
    blocked(call) {
      return (
        (this.init && !this.initializers.has(call)) ||
        [...this.locks.keys()].some((k) => k !== call)
      );
    }
    async enter(call) {
      while (this.blocked(call))
        await new Promise((resolve, reject) =>
          this.waiters.push({ resolve, reject }),
        );
      if (this.error) throw this.error;
    }
    wake() {
      const waiters = this.waiters.splice(0);
      for (const w of waiters) this.error ? w.reject(this.error) : w.resolve();
    }
    lock(call) {
      this.locks.set(call, (this.locks.get(call) ?? 0) + 1);
      return () =>
        watchdog(() => {
          const count = this.locks.get(call) - 1;
          if (count) this.locks.set(call, count);
          else this.locks.delete(call);
          this.wake();
        }, 0);
    }
    async output() {
      while (this.writes.size) await Promise.all([...this.writes]);
      if (this.error) throw this.error;
    }
    async storage(owner, op, key, value) {
      // The input lock is acquired synchronously before the first await.
      const unlock = this.lock(owner.call);
      const p = this.tail.then(() =>
        raw("do.storage", {
          object: owner.object,
          op,
          key: String(key),
          value,
        }),
      );
      this.tail = p.catch(() => {});
      if (op !== "get") {
        this.writes.add(p);
        p.catch((e) => {
          this.error = e;
          this.wake();
        });
      }
      try {
        return await p;
      } finally {
        this.writes.delete(p);
        unlock();
      }
    }
  }
  // Host completions and timers retain their initiating object's call identity.
  globalThis.__durableInput = async (promise) => {
    const owner = current;
    const value = await promise;
    if (owner) await owner.gate.enter(owner.call);
    return value;
  };
  globalThis.__durableOutput = async () => {
    if (current) await current.gate.output();
  };
  globalThis.__durableTimers = () => {
    for (const name of ["setTimeout", "setInterval"]) {
      const native = globalThis[name];
      globalThis[name] = (fn, ms, ...args) => {
        const owner = current;
        return native(
          () =>
            within(owner, () => {
              if (!owner) return fn(...args);
              owner.gate
                .enter(owner.call)
                .then(() => within(owner, () => fn(...args)));
            }),
          ms,
        );
      };
    }
  };
  globalThis.__gatewayDurable = async (module, env, ctx) => {
    watchdog = globalThis.setTimeout;
    __durableTimers();
    async function dispatch(message) {
      const { object, class: className, call, method } = message;
      let state = objects.get(object);
      if (!state) {
        const gate = new Gate();
        const owner = { object, call, gate };
        state = { gate, instance: null };
        objects.set(object, state);
        const storage = Object.freeze({
          async get(key) {
            const v = await gate.storage(current ?? owner, "get", key);
            return v === null ? undefined : decode(v);
          },
          put(key, value) {
            return gate.storage(current ?? owner, "put", key, encode(value));
          },
          delete(key) {
            return gate.storage(current ?? owner, "delete", key);
          },
        });
        const context = {
          id: Object.freeze({ toString: () => object }),
          storage,
          fetch: ctx.fetch,
          blockConcurrencyWhile(fn) {
            const previous = gate.init;
            const initializer = current ?? owner;
            gate.initializers.set(
              initializer.call,
              (gate.initializers.get(initializer.call) ?? 0) + 1,
            );
            const release = () => {
              const count = gate.initializers.get(initializer.call) - 1;
              if (count) gate.initializers.set(initializer.call, count);
              else gate.initializers.delete(initializer.call);
            };
            const work = within(initializer, async () => {
              if (previous) await previous;
              return fn();
            });
            let timer;
            const timeout = new Promise((_, reject) => {
              timer = watchdog(
                () => reject(new Error("DO initialization timed out")),
                30000,
              );
            });
            const pending = Promise.race([work, timeout]);
            gate.init = pending;
            pending.then(
              () => {
                clearTimeout(timer);
                release();
                if (gate.init === pending) gate.init = null;
                gate.wake();
              },
              (e) => {
                clearTimeout(timer);
                release();
                gate.error = e;
                gate.init = null;
                gate.wake();
                if (objects.get(object) === state) objects.delete(object);
              },
            );
            return pending;
          },
        };
        try {
          state.instance = within(
            owner,
            () => new module[className](context, env),
          );
        } catch (e) {
          objects.delete(object);
          throw e;
        }
      }
      const owner = { object, call, gate: state.gate };
      // The initializer's own I/O may resume; no RPC method runs before init.
      while (state.gate.init) await state.gate.init;
      while (state.gate.blocked(call)) await state.gate.enter(call);
      if (state.gate.error) throw state.gate.error;
      if (!allowed(method) || typeof state.instance[method] !== "function")
        throw new TypeError("unknown DO RPC method");
      const result = await within(owner, () =>
        state.instance[method](...decode(message.args)),
      );
      await state.gate.output();
      return encode(result);
    }
    for (;;) {
      const message = await raw("do.receive", {});
      dispatch(message).then(
        (value) => raw("do.reply", { call: message.call, value }),
        (error) =>
          raw("do.reply", {
            call: message.call,
            error: String(error?.message ?? error),
          }),
      );
    }
  };
})();
