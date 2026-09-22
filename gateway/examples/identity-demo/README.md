# 最小身份插件

演示 Core 用户、登录会话、Key 和四种路由鉴权，无额外 npm 依赖。不包含模型调用、部门或额度。

启动本轮构建的 Gateway，按实际配置设置 `GATEWAY_ADMIN_TOKEN`。在本目录打包并安装：

```sh
python3 -m zipfile -c /tmp/identity-demo.zip wrangler.toml index.js
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/zip' --data-binary @/tmp/identity-demo.zip \
  http://127.0.0.1:8080/__gateway/plugins
```

创建普通用户（替换示例密码）：

```sh
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"login":"alice","password":"replace-this-password","administrator":false}' \
  http://127.0.0.1:8080/__gateway/users
```

登录，保存响应中的 token 到 `GATEWAY_SESSION`：

```sh
curl -H 'Content-Type: application/json' \
  -d '{"login":"alice","password":"replace-this-password"}' \
  http://127.0.0.1:8080/__gateway/auth/login
```

用会话创建个人 Key，保存响应中的 key 到 `GATEWAY_API_KEY`：

```sh
curl -H "Authorization: Bearer $GATEWAY_SESSION" \
  -H 'Content-Type: application/json' -d '{"name":"IDE"}' \
  http://127.0.0.1:8080/__gateway/me/keys
curl -H "Authorization: Bearer $GATEWAY_API_KEY" \
  http://127.0.0.1:8080/identity/call
```

返回 `identity.userId`、`keyId`、`kind: "key"`、`administrator: false`。使用会话调用 `/identity/account`；使用平台管理员令牌或管理员会话调用 `/identity/admin`；`/identity/health` 无需认证。

普通用户会话访问 `/identity/admin` 返回 403；API Key 访问管理接口返回 401。请求头中的 `x-user-id` 不能覆盖宿主身份。完整的停用、过期、撤销和配置规则见 [身份文档](../../../docs/gateway-users-organizations-keys.md)。
