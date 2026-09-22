import { DurableObject } from "cloudflare:workers";
export const CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
export const credential = (env, id) => env.CREDS.get(env.CREDS.idFromName(id));
export const directory = (env) =>
  env.POOL.get(env.POOL.idFromName("directory"));
const account = (s) => ({
  id: s.id,
  state: s.state,
  expiresAt: s.expiresAt,
  accountId: s.accountId,
  cooldown: s.cooldown ?? 0,
});
const authBase = (env) => env.AUTH_BASE ?? "https://auth.openai.com";
function jwt(token) {
  if (!token) return {};
  const segment = token.split(".")[1];
  if (!segment) throw new Error("invalid id_token");
  const base64 = segment.replace(/-/g, "+").replace(/_/g, "/");
  const bytes = Uint8Array.from(
    atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4)),
    (c) => c.charCodeAt(0),
  );
  return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
}
function tokenState(id, old, data) {
  if (!data.access_token) throw new Error("OAuth response lacks access_token");
  const claims = jwt(data.id_token);
  const expires = Number(data.expires_in ?? 3600);
  if (!Number.isFinite(expires) || expires < 0)
    throw new Error("invalid token expiry");
  return {
    ...old,
    id,
    state: "active",
    access: data.access_token,
    refresh: data.refresh_token ?? old?.refresh,
    accountId:
      claims["https://api.openai.com/auth"]?.chatgpt_account_id ??
      claims.chatgpt_account_id ??
      old?.accountId,
    expiresAt: Date.now() + expires * 1000,
    cooldown: 0,
  };
}
export class Credential extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.state = await ctx.storage.get("state");
    });
  }
  async install(state) {
    await this.ctx.storage.put("state", state);
    this.state = state;
    return account(state);
  }
  async status() {
    return this.state ? account(this.state) : null;
  }
  async cooldown(seconds) {
    const state = { ...this.state, cooldown: Date.now() + seconds * 1000 };
    await this.ctx.storage.put("state", state);
    this.state = state;
  }
  async token() {
    if (!this.state || this.state.state === "dead")
      throw new Error("account unavailable");
    if (this.state.cooldown > Date.now())
      throw new Error("account cooling down");
    if (this.state.expiresAt > Date.now() + 60000) return this.state;
    if (!this.refreshing)
      this.refreshing = this.refresh().finally(() => {
        this.refreshing = null;
      });
    return this.refreshing;
  }
  async refresh() {
    const response = await this.ctx.fetch(authBase(this.env) + "/oauth/token", {
      method: "POST",
      proxy: this.state.proxy ?? "direct",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "refresh_token",
        client_id: CLIENT_ID,
        refresh_token: this.state.refresh,
      }),
    });
    const data = await response.json();
    if (!response.ok) {
      if (
        data.error === "refresh_token_reused" ||
        data.error?.code === "refresh_token_reused"
      )
        await this.install({ ...this.state, state: "dead" });
      throw new Error(
        `refresh failed: ${data.error?.code ?? data.error ?? response.status}`,
      );
    }
    const state = tokenState(this.state.id, this.state, data);
    await this.install(state);
    return state;
  }
  async maintain() {
    if (
      !this.state ||
      this.state.state === "dead" ||
      this.state.cooldown > Date.now()
    )
      return;
    await this.token();
  }
}
export class Pool extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.polling = new Map();
    this.registering = Promise.resolve();
  }
  async register(id) {
    const update = this.registering.then(async () => {
      const ids = (await this.ctx.storage.get("accounts")) ?? [];
      if (!ids.includes(id)) ids.push(id);
      await this.ctx.storage.put("accounts", ids);
      await this.env.CONFIG.put("accounts", JSON.stringify(ids));
      return ids;
    });
    this.registering = update.catch(() => {});
    return update;
  }
  async registerCall(id) {
    const work = this.registering.then(async () => {
      const ids = (await this.env.CONFIG.get("calls", { type: "json" })) ?? [];
      if (!ids.includes(id)) ids.push(id);
      await this.env.CONFIG.put("calls", JSON.stringify(ids));
    });
    this.registering = work.catch(() => {});
    return work;
  }
  async pruneCalls() {
    const work = this.registering.then(async () => {
      const ids = (await this.env.CONFIG.get("calls", { type: "json" })) ?? [];
      const kept = [];
      for (const id of ids)
        if (!(await this.env.CALLS.get(this.env.CALLS.idFromName(id)).expire()))
          kept.push(id);
      await this.env.CONFIG.put("calls", JSON.stringify(kept));
    });
    this.registering = work.catch(() => {});
    return work;
  }
  claimUsage(id) {
    const work = this.registering.then(async () => {
      const key = "usage:" + id;
      if (await this.env.CONFIG.get(key)) return false;
      await this.env.CONFIG.put(key, "1", { expirationTtl: 3600 });
      return true;
    });
    this.registering = work.catch(() => {});
    return work;
  }
  async login(proxy = "direct") {
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/usercode",
      {
        method: "POST",
        proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ client_id: CLIENT_ID }),
      },
    );
    const data = await response.json();
    if (!response.ok)
      throw new Error(`device authorization failed: ${response.status}`);
    if (!data.device_auth_id || !(data.user_code ?? data.usercode))
      throw new Error("invalid device authorization response");
    const id = crypto.randomUUID(),
      interval = Math.max(1, Number(data.interval ?? 5)),
      expires_at =
        Date.now() + Math.min(900, Number(data.expires_in ?? 900)) * 1000;
    if (!Number.isFinite(interval) || !Number.isFinite(expires_at))
      throw new Error("invalid device authorization timing");
    const state = {
      id,
      proxy,
      device_auth_id: data.device_auth_id,
      user_code: data.user_code ?? data.usercode,
      interval,
      expires_at,
      next: 0,
    };
    await this.ctx.storage.put("login:" + id, state);
    return {
      id,
      user_code: state.user_code,
      verify_url: data.verification_uri ?? authBase(this.env) + "/codex/device",
      expires_at,
      interval,
    };
  }
  async poll(id) {
    if (!this.polling.has(id))
      this.polling.set(
        id,
        this.pollOnce(id).finally(() => this.polling.delete(id)),
      );
    return this.polling.get(id);
  }
  async pollOnce(id) {
    const key = "login:" + id,
      state = await this.ctx.storage.get(key);
    if (!state || state.expires_at <= Date.now()) return { state: "expired" };
    if (state.done) return state.done;
    if (state.next > Date.now())
      return {
        state: "slow_down",
        retry_after: Math.ceil((state.next - Date.now()) / 1000),
      };
    state.next = Date.now() + state.interval * 1000;
    await this.ctx.storage.put(key, state);
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          device_auth_id: state.device_auth_id,
          user_code: state.user_code,
        }),
      },
    );
    const data = await response.json();
    if (!response.ok) {
      const code = data.error?.code ?? data.error;
      if (code === "expired_token" || response.status === 410)
        return { state: "expired" };
      if (code === "slow_down" || response.status === 429) {
        state.interval += 5;
        state.next = Date.now() + state.interval * 1000;
        await this.ctx.storage.put(key, state);
        return { state: "slow_down", retry_after: state.interval };
      }
      if (
        code === "authorization_pending" ||
        [403, 404].includes(response.status)
      )
        return { state: "pending", retry_after: state.interval };
      throw new Error(`device poll failed: ${code ?? response.status}`);
    }
    if (!data.authorization_code || !data.code_verifier)
      throw new Error("invalid device token response");
    const exchanged = await this.ctx.fetch(
      authBase(this.env) + "/oauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "authorization_code",
          client_id: CLIENT_ID,
          code: data.authorization_code,
          code_verifier: data.code_verifier,
          redirect_uri: authBase(this.env) + "/deviceauth/callback",
        }),
      },
    );
    const tokens = await exchanged.json();
    if (!exchanged.ok)
      throw new Error(`token exchange failed: ${exchanged.status}`);
    const claims = jwt(tokens.id_token);
    const accountId =
      claims["https://api.openai.com/auth"]?.chatgpt_account_id ??
      claims.chatgpt_account_id ??
      id;
    const installed = await credential(this.env, accountId).install(
      tokenState(accountId, { proxy: state.proxy }, tokens),
    );
    await this.register(accountId);
    state.done = { state: "done", account: installed };
    await this.ctx.storage.put(key, state);
    return state.done;
  }
}
