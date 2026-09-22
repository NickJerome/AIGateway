import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
// Replace only the runtime-owned base class; exercise the production methods.
const source = (
  await readFile(new URL("../src/auth.js", import.meta.url), "utf8")
).replace(
  'import { DurableObject } from "cloudflare:workers";',
  "class DurableObject { constructor(ctx, env) {this.ctx=ctx;this.env=env;} }",
);
const { Credential, Pool } = await import(
  "data:text/javascript;base64," + Buffer.from(source).toString("base64")
);
function context(
  entries = [],
  fetch = () => {
    throw Error("unexpected network");
  },
) {
  const rows = new Map(entries);
  let init;
  const ctx = {
    storage: {
      async get(k) {
        return structuredClone(rows.get(k));
      },
      async put(k, v) {
        rows.set(k, structuredClone(v));
      },
      async delete(k) {
        rows.delete(k);
      },
    },
    fetch,
    blockConcurrencyWhile(fn) {
      init = fn();
      return init;
    },
  };
  return { ctx, rows, initialized: () => init };
}
const state = {
  id: "acct",
  accountId: "acct",
  access: "old",
  refresh: "refresh-old",
  expiresAt: 0,
  state: "active",
  enabled: true,
};
test("refresh token rotation is retained and late 401 does not refresh a second time", async () => {
  let count = 0;
  const c = context([["state", state]], async () => {
    count++;
    return Response.json({
      access_token: "new",
      refresh_token: "refresh-new",
      expires_in: 3600,
    });
  });
  const account = new Credential(c.ctx, {});
  await c.initialized();
  await account.token(true, "old");
  await account.token(true, "old");
  assert.equal(count, 1);
  assert.equal(c.rows.get("state").refresh, "refresh-new");
  assert.equal((await account.status()).accountId, "acct");
});
test("invalid grant persists dead state and prevents repeated refresh", async () => {
  let count = 0;
  const c = context([["state", state]], async () => {
    count++;
    return Response.json({ error: "invalid_grant" }, { status: 400 });
  });
  const account = new Credential(c.ctx, {});
  await c.initialized();
  await assert.rejects(account.token(), /refresh failed/);
  await assert.rejects(account.token(), /unavailable/);
  assert.equal(count, 1);
  assert.equal(c.rows.get("state").state, "dead");
});
test("disabled accounts stay disabled during maintenance", async () => {
  const c = context([["state", { ...state, enabled: false }]]);
  const account = new Credential(c.ctx, {});
  await c.initialized();
  await account.maintain();
  await assert.rejects(account.token(), /unavailable/);
  assert.equal((await account.status()).enabled, false);
});
test("login cleanup uses supported storage get/put/delete and keeps active sessions", async () => {
  const c = context([
    ["logins", ["old", "new"]],
    ["login:old", { expires_at: 0 }],
    ["login:new", { expires_at: Date.now() + 60000 }],
  ]);
  const pool = new Pool(c.ctx, {});
  await pool.pruneLogins();
  assert.deepEqual(c.rows.get("logins"), ["new"]);
  assert.equal(c.rows.has("login:old"), false);
});
test("round robin and preferred account are deterministic", async () => {
  const c = context([["accounts", ["a", "b", "c"]]]);
  const pool = new Pool(c.ctx, {});
  assert.deepEqual(await pool.candidates(), ["a", "b", "c"]);
  assert.deepEqual(await pool.candidates(), ["b", "c", "a"]);
  assert.deepEqual(await pool.candidates("b"), ["b", "c", "a"]);
});
test("device polling honors local interval and upstream slow_down", async () => {
  let calls = 0;
  const c = context(
    [
      [
        "login:fixture",
        {
          expires_at: Date.now() + 60000,
          next: 0,
          interval: 1,
          device_auth_id: "d",
          user_code: "u",
          proxy: "direct",
        },
      ],
    ],
    async () => {
      calls++;
      return Response.json({ error: "slow_down" }, { status: 429 });
    },
  );
  const pool = new Pool(c.ctx, {});
  assert.equal((await pool.poll("fixture")).state, "slow_down");
  assert.equal((await pool.poll("fixture")).state, "slow_down");
  assert.equal(calls, 1);
  assert.equal(c.rows.get("login:fixture").interval, 6);
});
