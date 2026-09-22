# 插件

[Codex](codex/README.md)：可直接上传的 Codex HTTP Responses 插件，包含客户端 API Key 校验、OAuth 账号管理、刷新与多账号选择；安装包为 `codex/codex.zip`。

[Octopus](octopus/README.md)：无依赖的三协议转发插件，支持 Chat Completions、Responses、Anthropic Messages 的 JSON/SSE；使用 Core Key 鉴权，安装包为 `octopus/octopus.zip`。

本目录用于可独立构建、上传的插件源码与产物。安装后的运行目录由宿主的 `<data_dir>/plugins/<name>/` 管理，不能把源码目录直接当成已安装插件。

Core 已提供用户、Key、路由鉴权及只读 `env.IDENTITY`，见 [身份接口](../docs/gateway-users-organizations-keys.md) 和 [最小示例](../gateway/examples/identity-demo/README.md)。现有 Codex 插件暂保留原客户端鉴权，不能只替换凭证而不迁移代码。用户与 Key 可通过 [控制台](../console/README.md) 管理；上游 OAuth 凭证继续归插件，避免将特定提供商业务引入 Core。
