# Codex 插件

可上传到 AIGateway 的原生 Codex HTTP 插件。参考 CLIProxyAPI 的 Codex 请求规范化、OAuth 和 SSE 聚合方式实现，源码依据见 [NOTICE](NOTICE.md)。构建产物为 [codex.zip](codex.zip)，运行时无需 Node。

支持 OpenAI Responses JSON/SSE、compact、设备码登录、CLIProxyAPI/原生 Codex 凭证导入、共享刷新、多账号轮转与软会话粘性。客户端 API Key 和管理员令牌分开，未配置时拒绝访问。

本插件没有用户登录、注册、余额、计费、用户管理页面或管理前端。`CLIENT_API_KEYS[].id` 是调用方标签，不是用户账号。Chat Completions、Anthropic 转换和 WebSocket/realtime 没有接入这个插件；Core 跨协议实现的已知问题仍见 [评审](../../reviews/axonhub-protocol-review/README.md)。

## 安装与配置

从仓库根目录上传：

```sh
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/zip' --data-binary @plugins/codex/codex.zip \
  http://127.0.0.1:8080/__gateway/plugins
```

将以下内容保存为本地私有配置 `codex.private.json`，替换令牌与模型名称：

```json
{
  "vars": {
    "ADMIN_TOKEN": "替换为插件管理员令牌",
    "CLIENT_API_KEYS": [
      {"id": "alice", "key": "替换为客户端APIKey", "enabled": true, "models": ["my-codex"]}
    ],
    "MODEL_MAP": {"my-codex": "替换为账号支持的真实模型名"},
    "MODELS": ["my-codex"]
  },
  "trace_body": false
}
```

```sh
curl -X PUT -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' --data-binary @codex.private.json \
  http://127.0.0.1:8080/__gateway/plugins/codex/config
```

这是完整覆盖，更新 Key 时一并保留已有 vars。`models` 省略表示允许该 Key 请求所有模型；设置空数组表示全部禁止。模型权限按客户端请求名称校验，再应用 MODEL_MAP。`MODELS` 是展示列表，不代表从上游探测得到的能力。删除 Key 或设 `enabled:false` 后，下一次请求拒绝；配置更新还会按宿主既有机制取消旧请求。

可选变量：`UPSTREAM`（默认 `https://chatgpt.com/backend-api/codex`，不含 `/responses`）、`AUTH_BASE`（默认 `https://auth.openai.com`）、`ORIGINATOR`、`UPSTREAM_USER_AGENT`。默认 User-Agent 标识本插件，没有模拟完整 CLIProxyAPI 的 TLS 指纹或客户端环境。

不要同时启用占用 `/codex/*` 的旧 `codex-pool` 示例。宿主拒绝相同路由冲突；正式插件与示例使用不同存储命名空间，不会自动迁移示例账号。

## 鉴权层次

| 凭证 | 用途 | 保存位置 |
|---|---|---|
| `GATEWAY_ADMIN_TOKEN` | 宿主 `/__gateway/*`，安装、配置与运维查询 | 宿主环境变量 |
| 插件 `ADMIN_TOKEN` | `/codex/admin/*`，登录、导入、启停账号与查询用量 | SQLite 中的插件运行覆盖 |
| `CLIENT_API_KEYS` | 推理、compact、模型列表 | SQLite 中的插件运行覆盖 |
| 上游 access/refresh token | 插件代表账号调用 Codex、刷新登录 | Credential DO，底层同一个 SQLite |

客户端使用 `Authorization: Bearer <客户端Key>`。不接受查询参数里的 Key，不把客户端 Authorization/Cookie 当作上游凭证。管理员令牌不能用于推理，客户端 Key 不能管理账号。Key 的 `id` 写入 USAGE 的 caller 字段；未实现按用户的账单或配额。

运行变量和上游凭证沿用项目的明文持久化设计。`trace_body:false` 只关闭正文采集，宿主仍可能保存 headers 等元数据；这不是密钥脱敏开关。账号列表只返回状态，绝不返回 access/refresh token。

## 添加账号

### 导入现有凭证

管理员将自己的凭证文件 POST 到导入入口：

```sh
curl -H "Authorization: Bearer $CODEX_PLUGIN_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' --data-binary @account.private.json \
  http://127.0.0.1:8080/codex/admin/accounts/import
```

接受 CLIProxyAPI 风格的 `access_token`、`refresh_token`、`account_id`、`expired`（日期字符串），以及原生 Codex 的 `tokens` 包装。也可传 `expires_at`（UTC 毫秒）和 `proxy`。account_id 可从 token claims 读取；无明确到期值或可用 claims 时只按一小时处理，建议提供真实到期值。缺少 refresh token 时只能使用当前 access token，到期需重新导入或登录。

导入只由管理员授权，JWT claims 用于提取身份元数据，不充当客户端鉴权。插件不会自动读取服务器上的 CLI 凭证文件。按 account_id 重复导入会替换该账号凭证并重新启用。

### 设备码登录

```sh
curl -X POST -H "Authorization: Bearer $CODEX_PLUGIN_ADMIN_TOKEN" \
  http://127.0.0.1:8080/codex/admin/accounts/login
```

得到 `id`、`user_code`、`verify_url`、`interval` 后，由管理员打开验证页授权；按间隔调用：

```sh
curl -X POST -H "Authorization: Bearer $CODEX_PLUGIN_ADMIN_TOKEN" \
  http://127.0.0.1:8080/codex/admin/accounts/login/登录ID/poll
```

支持 pending、slow_down、expired、done。可在 login 请求通过 URL 编码的 `proxy` 参数指定代理。刷新请求复用账号代理。账号共享单次刷新；401 最多刷新并重试该账号一次，429 按 Retry-After 冷却后尝试其他账号。invalid_grant/reused 刷新错误将账号标记 dead，需重新登录或导入。

## 接口

| 方法与路径 | 用途 |
|---|---|
| `POST /v1/responses` | 原生 Responses，JSON 或 SSE |
| `POST /v1/responses/compact` | 原生压缩响应 |
| `GET /v1/models` | 返回配置模型列表并按 Key 权限过滤 |
| `GET /codex/admin/accounts` | 账号状态 |
| `POST /codex/admin/accounts/import` | 导入账号 |
| `PUT /codex/admin/accounts/{id}/enabled` | `{ "enabled": false }` 停用；true 启用 |
| `POST /codex/admin/accounts/login` | 启动设备码登录 |
| `POST /codex/admin/accounts/login/{id}/poll` | 轮询授权 |
| `GET /codex/admin/usage?since=<UTC毫秒>` | 查询已提交用量 |

推理、compact、models 另有 `/codex/responses`、`/codex/responses/compact`、`/codex/models` 别名。没有浏览器前端或登录 Cookie。

```sh
curl -H "Authorization: Bearer $CODEX_CLIENT_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"my-codex","input":"Reply OK","stream":false}' \
  http://127.0.0.1:8080/v1/responses
```

## Codex 行为与边界

- 后端 `/responses` 始终使用 `stream:true`、`store:false`。客户端非流式请求聚合到 Responses 终态；从 output_item.done 补全省略的最终 output。原生加密 reasoning 和工具 item 保留。
- 请求字符串 input 改为消息数组，system 角色改 developer，instructions 缺失时补空字符串。固定包含 reasoning.encrypted_content，保留其他 include。
- 参考实现会移除后端不支持的采样/token-limit/truncation 等字段：具体清单在 `src/codex.js` 的 prepare 中。**客户端传入 token 上限不构成此后端的输出限制**。service_tier 仅保留 priority；不自动加入图像工具或修改工具参数语义。
- `/responses/compact` 使用独立非流式路径。compact 的 `stream:true` 明确拒绝。
- `previous_response_id`、conversation、background 明确拒绝。当前 Codex HTTP 适配要求请求携带完整历史，不会悄悄删掉 ID 并丢失上下文。
- prompt_cache_key、Session-Id 或 Thread-Id 用于软粘性，账号不可用可以换账号。映射按调用方 id 隔离，TTL 一小时；不是服务端会话存储。
- SSE 按帧增量透传，必须出现 Responses 语义终态，裸 `[DONE]` 不算成功。中断产生错误帧；非流式中断返回 502。输出已开始后不切换账号重放。
- 单个 SSE 帧上限 1 MiB，非流式重建 output 上限 8 MiB，宿主其余额度仍适用。
- 每五分钟维护凭证并清理设备码会话。用量按账号与 response ID 去重，异步批写；不是结算系统。
- 尚未使用真实 ChatGPT/Codex OAuth 账号验收。本轮不使用此前测试第三方 API 的 Key 冒充 Codex OAuth。

## 开发与验证

```sh
cd plugins/codex
npm ci --ignore-scripts
npm test
npm run package
python3 tests/smoke.py
```

`smoke.py` 需要 `gateway/target/release/gateway` 已构建；仅启动临时 Gateway 和本地 Codex/OAuth mock，退出后清理。检查的是 ZIP 实际上传、QuickJS 执行、鉴权、请求形状、SSE/JSON、刷新、轮换和持久化。[验证记录](../../reviews/codex-plugin-validation/README.md)。
