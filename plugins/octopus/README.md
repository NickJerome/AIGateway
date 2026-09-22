# Octopus 插件

一个无 npm 依赖的 JS 插件，直接调用 Octopus 的三种原生 API。默认模型 `qwen3.8-flash-next`，支持普通 JSON 和 SSE 流式响应，保留上游响应正文、状态码、usage 和错误内容。

默认路由：

| 网关入口 | Octopus 上游 |
| --- | --- |
| `POST /octopus/v1/chat/completions` | `/v1/chat/completions` |
| `POST /octopus/v1/responses` | `/v1/responses` |
| `POST /octopus/v1/messages` | `/v1/messages` |
| `GET /octopus/v1/models` | 返回配置的模型 |

前缀用于与现有 Codex 插件共存。所有入口声明 `auth = "key"`，由 Core 校验控制台创建的个人或服务 Key。上游 API Key 由插件配置提供，不需要发给客户端。

## 打包与安装

```sh
python3 plugins/octopus/package.py
export GATEWAY_URL='http://127.0.0.1:8080'
# GATEWAY_ADMIN_TOKEN：部署时的平台管理令牌，也可使用管理员登录会话。
# OCTOPUS_API_KEY：你的 Octopus 上游 Key。
curl --fail-with-body "$GATEWAY_URL/__gateway/plugins" \
  -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/zip' \
  --data-binary @plugins/octopus/octopus.zip
```

配置上游：

```sh
python3 - <<'PY' | curl --fail-with-body -X PUT \
  "$GATEWAY_URL/__gateway/plugins/octopus/config" \
  -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' --data-binary @-
import json, os
print(json.dumps({
    "vars": {
        "UPSTREAM": "https://octopus.wangjerome.com",
        "MODEL": "qwen3.8-flash-next",
        "API_KEY": os.environ["OCTOPUS_API_KEY"],
    },
    "trace_body": False,
}))
PY
```

`API_KEY` 未配置时，推理返回 503。安装包不包含上游 Key。此配置示例关闭正文 trace；配置中的 Key 由现有网关配置存储保存。

## 调用

在控制台创建 Key，将它设为 `GATEWAY_KEY`，然后调用：

```sh
curl "$GATEWAY_URL/octopus/v1/chat/completions" \
  -H "Authorization: Bearer $GATEWAY_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next","messages":[{"role":"user","content":"你好"}]}'

curl -N "$GATEWAY_URL/octopus/v1/responses" \
  -H "Authorization: Bearer $GATEWAY_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next","input":"你好","stream":true}'

curl "$GATEWAY_URL/octopus/v1/messages" \
  -H "Authorization: Bearer $GATEWAY_KEY" \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2023-06-01' \
  -d '{"model":"qwen3.8-flash-next","max_tokens":256,"messages":[{"role":"user","content":"你好"}]}'
```

OpenAI 兼容客户端的 Base URL 使用 `http://网关地址/octopus/v1`；Anthropic 兼容客户端的 Base URL 使用 `http://网关地址/octopus`。**Core 当前只识别 `Authorization: Bearer <网关 Key>`**，因此 Anthropic 客户端需要使用 Bearer/auth_token 配置；仅发送 `x-api-key` 不能通过 Core 鉴权。转发上游时，插件设置 Octopus 自己的凭证，不透传客户端凭证。

`model` 省略时使用配置的模型；指定其他模型时返回 400。其他请求字段保留，包括 tools、thinking 等，具体能力由上游决定。Chat Completions 流式请求会设置 `stream_options.include_usage=true`，让上游返回用量。响应字节保持原样，插件在转发过程中解析 usage 并通过 `env.USAGE.record` 上报给 Core，不维护独立额度账本。

如果需要直接使用 `/v1/...`，将 `wrangler.toml` 中四个路由的 `/octopus` 前缀删除后重新打包安装即可，JS 已兼容两种路径。安装前需释放其他插件已注册的相同路由，不能与 Codex 的 `/v1/responses`、`/v1/models` 重复注册。

## 日志与用量

Core Web 左侧的“调用记录、日志、用量”是统一入口，按插件筛选即可。调用记录由 Core 自动创建；日志来自插件的 `console`；用量来自 manifest 声明的 `USAGE` binding。Octopus 打印协议和上游 HTTP 状态，不打印 Key、请求正文或用户对话。

普通 JSON 读取 `usage`；SSE 分别读取 Chat Completions 最后的 usage、Responses 的 `response.completed.response.usage`、Anthropic 的 `message_start` 与 `message_delta` usage。在终止事件或正常 EOF 时只上报一次，累计输出取最新值。不会完整缓冲 SSE，也不会为了统计额外复制一条不受消费速度约束的流。

上报 `model`、`protocol`、`stream`、`complete`、调用者 user/key ID，以及实际存在的 `input_tokens`、`output_tokens`、`total_tokens`、`cache_read_tokens`、`cache_write_tokens`、`reasoning_tokens`。Anthropic 的输入字段将未缓存输入、缓存读取和缓存创建相加；其他协议的缓存数是 input 的细项，不再次加到总量中。缺失字段不补零、不估算价格。Core 用量页每次上报单独显示一条，包含模型、协议、各类 token 和请求 ID。

HTTP 错误不生成成功用量；成功响应缺少 usage 会打印 warning。已知 usage 但缺少 SSE 终止事件时，正常 EOF 上报 `complete=false`；客户端中途取消且尚未收到最终 usage，无法保证统计到上游之后产生的消耗。解析缓存设有上限，过大的 JSON 或 SSE 事件跳过后续解析并打印 warning，转发仍继续；已取得的 usage 仍可上报。关闭 `trace_body` 不影响日志或 usage。

## 测试

```sh
cargo build --manifest-path gateway/Cargo.toml --locked
node --test plugins/octopus/tests/usage.test.mjs
python3 plugins/octopus/tests/smoke.py
# 使用环境中的测试 Key，真实调用三种协议的 JSON 和 SSE，共 6 次推理：
python3 plugins/octopus/tests/smoke.py --live
```

测试自动启动临时网关、上传 ZIP、创建服务 Key，结束后删除临时数据库。默认测试使用本地 mock，验证真实宿主内的三协议转发、首块流式返回、凭证隔离、错误透传、Key 撤销、console 日志、6 条实际落库 usage 及按协议汇总。`--live` 需要 `OCTOPUS_API_KEY` 环境变量，使用默认 Octopus 地址和模型，不打印 Key 或请求正文。可通过 `GATEWAY_BIN` 指定其他已编译网关二进制。
