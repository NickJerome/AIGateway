# Codex pool 示例

这是供运行时测试使用的示例，推理路由未校验客户端 API Key。实际 HTTP 接入请使用带客户端鉴权的[正式 Codex 插件](../../../plugins/codex/README.md)。

Hono quick + ESM 插件，使用 CONFIG KV、Credential/Pool/Call DO、IR 和 USAGE。ZIP 包已构建，运行无需 npm。默认 HTTP 上游为 `https://chatgpt.com/backend-api/codex/responses`；设备码授权为 `https://auth.openai.com`。这些端点依据本地 CLIProxyAPI 参考实现，测试只访问本地 mock。

## 安装和配置

宿主以设置了 `GATEWAY_ADMIN_TOKEN` 的环境启动，然后上传 ZIP。插件管理令牌与宿主管理令牌是两项配置：

```sh
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/zip' --data-binary @codex-pool.zip \
  http://127.0.0.1:8080/__gateway/plugins
```

将以下 JSON 保存成私有的 `overrides.json`，替换其中的示例令牌：

```json
{"vars":{"ADMIN_TOKEN":"替换为插件管理员令牌"},"trace_body":true}
```

```sh
curl -X PUT -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' --data-binary @overrides.json \
  http://127.0.0.1:8080/__gateway/plugins/codex-pool/config
```

config 替换完整运行覆盖。修改时应一并保留已有的自定义 vars/bindings。可覆盖 `UPSTREAM`、`AUTH_BASE`、`REALTIME_UPSTREAM`、`REALTIME_BOOTSTRAP`、`MODEL_MAP`；后者为原模型名到目标名的 JSON 对象。令牌、凭证和真实代理认证信息不放入 ZIP。

## 使用

`/codex/admin/*` 需要插件的 Bearer token：

- `POST /codex/admin/accounts/login?proxy=...` 返回设备码、验证地址、到期时间和轮询间隔。未传 proxy 时直连，含特殊字符的代理 URL 需要 URL 编码。
- `POST /codex/admin/accounts/login/{id}/poll` 返回 pending/slow_down、done，或 410 过期。按 retry_after 等待后再轮询。
- `GET /codex/admin/accounts` 查看状态，不返回 access/refresh token。
- `GET /codex/admin/usage?since=<UTC毫秒>` 返回已经提交的用量聚合。

`POST /codex/responses` 接收 OpenAI Responses JSON，支持 SSE。同账号并发刷新共享 Promise；refresh token 省略时保留旧值，reused 标记 dead。401/429 在发送客户端响应前冷却并换账号；其他状态透传。`previous_response_id` 必须回到原账号，找不到或账号不可用时明确失败；软会话键可以降级，USAGE 记录 fallback 计数。ID 映射在该事件发给客户端之前 FULL 写入 KV。

response ID、软会话映射和用量去重标记的 TTL 为 3600 秒。用量异步 NORMAL 批写，异常进程退出可能丢尾批；记录数统计的是记录行，并不自动等于业务请求数。

- `GET /codex/realtime`：原生 WebSocket 透传；`call_id` 查询参数存在时转 sideband。
- `POST /codex/realtime/calls`：转发 SDP bootstrap，改写 Location。
- `GET /codex/realtime/calls/{id}`：固定账号的 sideband；同一 call 只准一个连接。

关闭回调通过 waitUntil 释放 Call 租约。每五分钟 cron 刷新凭证并清理过期 call；进程异常退出后的租约按 expiresAt 清理。这里只转发信令/WS，不中继 WebRTC 音视频。

## 开发与本地验证

```sh
npm ci --ignore-scripts
npm run package
cd ../..
cargo test --locked --test codex_e2e --test realtime_e2e
```

Hono 4.13.8、esbuild 0.25.12 已锁定；构建输出为 `dist/index.mjs`，只有 `cloudflare:workers` 保留外部导入。Hono MIT 许可在 `HONO-LICENSE` 并随 ZIP 提供。

冻结 mock 覆盖 100 并发一次刷新、硬/软粘性、401/429、dead、重启与重装、登录各分支、SSE 断开 100ms 内取消、call 互斥/释放和原生二进制透传。实际账号连通性没有作为 CI 成功条件。
