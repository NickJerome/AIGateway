import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn } from 'node:child_process'
const dir = await mkdtemp(join(tmpdir(), 'aigateway-console-test-'))
const config = join(dir, 'gateway.toml')
await writeFile(
  config,
  `listen = "127.0.0.1:19081"\ndata_dir = ${JSON.stringify(dir)}\nthreads = 1\nadmin_token_env = "CONSOLE_TEST_ADMIN"\n`,
)
const child = spawn(resolve('../gateway/target/debug/gateway'), ['run', '--config', config], {
  stdio: 'inherit',
  env: { ...process.env, CONSOLE_TEST_ADMIN: 'console-isolated-test-bootstrap' },
})
let stopping = false
const stop = () => {
  if (!stopping) {
    stopping = true
    child.kill('SIGTERM')
  }
}
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
child.on('error', async (error) => {
  console.error(error.message)
  await rm(dir, { recursive: true, force: true })
  process.exit(1)
})
child.on('exit', async (code) => {
  await rm(dir, { recursive: true, force: true })
  process.exit(stopping ? 0 : (code ?? 1))
})
