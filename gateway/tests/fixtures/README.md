# Test fixtures

All fixtures are vendored into the repository; the test suite never touches
the network.

## `hono-quick-4.13.8.mjs`

Single-file esbuild bundle of the real Hono framework, used by the Hono
acceptance tests in `tests/web_api.rs`.

- Source: npm `hono@4.13.8` (`hono/quick` preset plus the `hono/bearer-auth`,
  `hono/cors` and `hono/logger` middleware subpaths).
- Build: `esbuild entry.mjs --bundle --format=iife --platform=neutral`, where
  `entry.mjs` imports the four modules and assigns them to
  `globalThis.HonoQuick` (the runtime loads a single module per plugin, so the
  bundle exposes a global instead of ESM exports; the IIFE format keeps the
  concatenated module scope clean).
- sha256: `42e7737ca802fba960b6f0109cc49cd6e1590ab7bc6de1a6ee6754bec82c1a69`
- The bundle is self-contained (no static imports). Its one dynamic
  `import("cloudflare:workers")` sits behind a `navigator.userAgent` check
  that is never true in this runtime, so it is never evaluated.

## `workerd/`

Assertions ported from the workerd test suite; see `workerd/README.md` for
the porting manifest and the list of branches that do not apply here.
