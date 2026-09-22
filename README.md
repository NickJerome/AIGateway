# AIGateway

面向小团队的单进程插件网关：Rust 宿主、静态链接 QuickJS、一个 SQLite 数据库。已构建插件运行时不需要 Node、npm 或 Wrangler。

HTTP 请求各自创建 Engine；插件的 DO Engine 按工作线程常驻。源码与编译产物由插件管理负责。没有 Engine 池或分布式协调；用户与 Key 管理页面位于独立的 Vue 控制台。

## 运行

```sh
cd gateway
cargo build --release --locked
cp gateway.example.toml gateway.toml
export GATEWAY_ADMIN_TOKEN='替换为自己的管理员令牌'
./target/release/gateway run --config gateway.toml
```

发布包内可直接执行 `./gateway run --config gateway.toml`。二进制适用于 Linux x86_64/glibc；QuickJS、SQLite 已编入二进制，仍依赖系统 libc/libm/libgcc。首次使用管理员环境令牌创建管理员用户；后续可用管理员登录会话管理。指定了管理员环境变量但值为空时启动失败。

ZIP 根目录包含 `wrangler.toml` 和打包后的 ESM；Core 只内建 `cloudflare:workers` 的 DurableObject 基类。上传、编译、启用通过同一个管理接口完成：

```sh
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/zip' --data-binary @../plugins/codex/codex.zip \
  http://127.0.0.1:8080/__gateway/plugins
```

Core 已提供用户登录、个人/服务 Key、路由鉴权和只读 `env.IDENTITY`。部门、额度和模型权限留给 JS。用户、Key 与插件管理已提供 [Vue 3 + shadcn-vue 控制台](console/README.md)，开发时运行 `cd console && npm ci && npm run dev`，访问 `/console/`。

现有 Codex 插件仍需配置插件管理员令牌、客户端 API Key 和上游账号，见 [插件说明](plugins/codex/README.md)。未配置客户端 Key 时拒绝推理请求；本轮未自动迁移它的旧鉴权。Codex OAuth 账号验收使用本地 mock，尚未连接真实账号。

## 管理接口

下列平台接口接受宿主管理员 Bearer token 或管理员登录会话，不接受推理 API Key。
| 方法与路径 | 用途 |
|---|---|
| `GET /__gateway/plugins` | 插件状态 |
| `GET /__gateway/plugins/{name}/config` | 管理员读取默认变量与运行覆盖配置；不缓存 |
| `POST /__gateway/plugins` | 上传 ZIP；首次 201、重装 200 |
| `PUT /__gateway/plugins/{name}/config` | 替换运行覆盖 `vars`、KV `bindings`、`trace_body` |
| `PUT /__gateway/plugins/{name}/enabled` | JSON `{ "enabled": true/false }` 启停 |
| `DELETE /__gateway/plugins/{name}` | 卸载代码；保留业务数据 |
| `GET /__gateway/usage/records`、`GET /__gateway/logs`、`GET /__gateway/traces` | 全部插件的逐条记录，支持相同的时间和游标参数；每行含 `plugin_id` |
| `GET /__gateway/plugins/{name}/usage/records?order=desc&limit=50&before_id=...` | 原始 usage 明细，支持 since 和游标分页 |
| `GET /__gateway/plugins/{name}/usage?since=...&groupBy=credential,model` | 已提交聚合 |
| `GET /__gateway/plugins/{name}/logs?limit=100&after_id=...` | 日志分页 |
| `GET /__gateway/plugins/{name}/traces?limit=100&after_id=...` | trace 列表；`order=desc&before_id=...` 从最新向前翻页 |
| `GET /__gateway/plugins/{name}/traces/{id}` | trace 元数据 |
| `GET /__gateway/plugins/{name}/traces/{id}/body?side=request` | 按顺序流式读取正文；也支持 response |
| `GET /__gateway/metrics` | 活动请求/DO/WS、JS/宿主额度、遥测缓冲与失败计数 |

调用记录由 Core 在插件执行入口创建，全局 `fetch()` 和 `ctx.fetch()` 的上游调用都会自动关联，不要求插件显式调用 traces API。鉴权失败、路由未匹配、容量拒绝和请求正文读取失败发生在 trace 创建之前，不包含在此列表。`complete` 表示响应传输是否完成，HTTP 4xx/5xx 也可能传输完成。管理端遥测响应带 `Cache-Control: no-store`。

列表默认按 ID 正序，保留 `after_id`；控制台使用 `order=desc` 与 `before_id` 游标，每页 50 条。响应另含 `has_more`，每行包含 `plugin_id`。控制台默认选择“全部插件”，合并展示记录但不合计数值，也可筛选单个插件。Core Web 侧栏的“调用记录”支持按插件选择、元数据、上游调用和按需正文预览（最多 64 KiB）；“日志”展示 console 输出，“用量”按时间倒序逐条展示插件上报的 usage。插件工作台提供带插件筛选的快捷链接。

`trace_body:false` 对该插件关闭正文采集，仍记录元数据与 hash。默认完整 trace 明文保存原始 headers/body。有效保留字节限额包含日志和 trace，usage 使用单独保留期；SQLite 空闲页与 WAL 文件大小不等于有效 payload。

## 验证与限制

```sh
gateway/scripts/verify.sh
cargo test --manifest-path gateway/Cargo.toml --release --locked \
  --test process --test codex_e2e --test realtime_e2e
cargo test --manifest-path gateway/Cargo.toml --release --locked \
  --test ws_capacity --test storage_capacity -- --ignored --nocapture
```

HTTP 容量脚本为 `gateway/scripts/capacity.py`，只使用 Python 标准库。CI 文件是本地提供的检查入口，未代替远程仓库执行记录。

模块全局不跨普通请求保存；`waitUntil` 有限时长；原生同步调用不能被 JS 中断器强行打断。SQLite 是单进程持久层，重装会取消旧请求/DO/cron/两类 WS，不提供零停机替换。异常退出可能丢失遥测尾批，已确认 KV/DO 写入采用 FULL 提交。完整 WPT、Blob/BYOB、DO SQL/alarm/hibernation、WS 压缩/半开/RFC8441 等保持 v2 范围。

数据库连接池入口在 `gateway/src/storage.rs`；有序 SQL 升级在 `storage/schema.rs` 与 `storage/001.sql`–`004.sql`；KV 在 `storage/kv.rs`，插件注册和覆盖配置在 `plugins/persistence.rs`。身份 SQL 位于 `identity/repository.rs`，遥测读写位于 `telemetry/query.rs` 与 `telemetry/writer.rs`。不引入 ORM 或通用 Repository。打开数据库本身不再修改插件/trace 运行状态，恢复分别由 PluginManager/Telemetry 初始化负责。Rust 内部 API 的 `Overrides`、`PluginRecord` 移至 `gateway::plugins`；HTTP/JS 接口和数据库版本不变。

普通请求和管理上传的正文读取使用 `[http].idle_timeout_ms` 空闲超时，超时返回 408 并释放容量；身份请求另有 10 秒总时限。此空闲超时不等于整个上传的总时限。

当前模块边界、Cloudflare 源码参考与编码约定见 [Rust 架构与编码约定](docs/rust-architecture.md)。此前的问题修复与验证见 [Gateway 审查记录](reviews/gateway-audit-2026-09-22/README.md)。
