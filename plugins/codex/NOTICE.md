# Reference and third-party notices

This plugin adapts Codex HTTP integration behavior from the local CLIProxyAPI snapshot at commit `b773607e3e7756dc6020a291825e4eb08899595a`:

- `internal/translator/codex/openai/responses/codex_openai-responses_request.go`
- `internal/runtime/executor/codex_executor_request.go`
- `internal/runtime/executor/codex_executor_execute.go`
- `internal/runtime/executor/codex_executor_auth.go`
- `internal/auth/codex/` and `sdk/auth/codex_device.go`

The CLIProxyAPI MIT notice is included in CLIProxyAPI-LICENSE. Hono is bundled under the MIT license included in HONO-LICENSE. OAuth state management is adapted from this repository's codex-pool example. This is a scoped HTTP Responses plugin, not a port of the entire CLIProxyAPI server.
