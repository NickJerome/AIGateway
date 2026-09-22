import { test, expect, type APIRequestContext, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
const password = 'plugin-console-test-password'
const bootstrap = { Authorization: 'Bearer console-isolated-test-bootstrap' }
async function login(page: Page, request: APIRequestContext, admin = true) {
  const login = `plugin-admin-${Date.now()}`
  const res = await request.post('/__gateway/users', {
    headers: bootstrap,
    data: { login, password, administrator: admin },
  })
  expect(res.status()).toBe(201)
  await page.goto('login')
  await page.getByLabel('用户名', { exact: true }).fill(login)
  await page.getByLabel('密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '登录控制台' }).click()
  await expect(page.getByTestId('page-scroll-area')).toBeVisible()
}
function fixture(
  name: string,
  source = 'export default {fetch(r,env){return Response.json({model:env.MODEL})}}',
  extra = '',
) {
  return execFileSync('python3', [
    '-c',
    `import io,sys,zipfile
name=sys.argv[1]
b=io.BytesIO()
with zipfile.ZipFile(b,'w') as z:
 z.writestr('wrangler.toml',f'name="{name}"\\nmain="index.js"\\nroutes=["/{name}/*"]\\n[vars]\\nMODEL="initial-model"\\nAPI_KEY="fixture-hidden-key"\\n' + sys.argv[3])
 z.writestr('index.js',sys.argv[2])
sys.stdout.buffer.write(b.getvalue())`,
    name,
    source,
    extra,
  ])
}

test('安装包、动态侧栏入口、独立工作台、配置、启停及卸载', async ({ page, request }) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await login(page, request)
  await page.getByRole('link', { name: '插件管理', exact: true }).click()
  const name = `ui-plugin-${Date.now()}`
  await page.getByRole('button', { name: '安装插件', exact: true }).first().click()
  await page
    .getByLabel('插件安装包')
    .setInputFiles({ name: 'fixture.zip', mimeType: 'application/zip', buffer: fixture(name) })
  await page.getByRole('button', { name: '上传并安装' }).click()
  await expect(page.getByRole('heading', { name, exact: true })).toBeVisible()
  await expect(page.getByRole('link', { name: new RegExp(name) })).toBeVisible()
  await expect(page.getByRole('cell', { name: `/${name}/*`, exact: true })).toBeVisible()
  const configuration = await request.get(`/__gateway/plugins/${name}/config`, {
    headers: bootstrap,
  })
  expect(configuration.status()).toBe(200)
  expect(configuration.headers()['cache-control']).toBe('no-store')
  expect(await configuration.json()).toMatchObject({ defaults: { MODEL: 'initial-model' } })
  const listed = await request.get('/__gateway/plugins', { headers: bootstrap })
  expect(await listed.text()).not.toContain('fixture-hidden-key')
  await page.getByRole('tab', { name: '运行配置' }).click()
  await expect(page.getByLabel('API_KEY 的值', { exact: true })).toHaveAttribute('type', 'password')
  await page.getByLabel('MODEL 的值', { exact: true }).fill('updated-model')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '保存并应用' }).click()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  await page.reload()
  await page.getByRole('tab', { name: '运行配置' }).click()
  await expect(page.getByLabel('MODEL 的值', { exact: true })).toHaveValue('updated-model')
  await expect(page.getByLabel('API_KEY 的值', { exact: true })).toHaveValue('fixture-hidden-key')
  const live = await request.get(`http://127.0.0.1:19081/${name}/test`)
  expect(await live.json()).toEqual({ model: 'updated-model' })
  await page.getByRole('button', { name: '停用插件', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '停用插件', exact: true }).click()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  expect((await request.get(`http://127.0.0.1:19081/${name}/test`)).status()).toBe(503)
  await page.getByRole('button', { name: '启用插件', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '启用插件', exact: true }).click()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  expect((await request.get(`http://127.0.0.1:19081/${name}/test`)).status()).toBe(200)
  await page.getByRole('link', { name: '查看日志', exact: true }).click()
  await expect(page.getByText('暂无日志', { exact: true })).toBeVisible()
  await page.goto(`plugins/${name}`)
  await page.getByRole('tab', { name: '概览与接口' }).click()
  await page.getByRole('button', { name: '卸载插件', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '卸载插件', exact: true }).click()
  await expect(page).toHaveURL(/\/plugins$/)
  await expect(page.getByRole('link', { name: new RegExp(name) })).toHaveCount(0)
  expect(errors).toEqual([])
})

test('普通用户无法访问插件管理或读取上游配置', async ({ page, request }) => {
  await login(page, request, false)
  await expect(page.getByRole('link', { name: '插件管理', exact: true })).toHaveCount(0)
  await page.goto('plugins')
  await expect(page).toHaveURL(/my-keys$/)
  await page.goto('plugins/absent')
  await expect(page).toHaveURL(/my-keys$/)
  const statuses = await page.evaluate(async () => {
    return Promise.all(
      [
        'config',
        'traces',
        'traces/missing',
        'traces/missing/body',
        'logs',
        'usage',
        'usage/records',
      ]
        .map((path) => `plugins/absent/${path}`)
        .concat(['logs', 'traces', 'usage/records'])
        .map(async (path) => {
          const r = await fetch(`/__gateway/${path}`, {
            headers: { Authorization: 'Bearer ' + sessionStorage.getItem('aigateway.session') },
          })
          return r.status
        }),
    )
  })
  expect(statuses).toEqual([403, 403, 403, 403, 403, 403, 403, 403, 403, 403])
  for (const path of ['traces', 'logs', 'usage', 'usage/records']) {
    await page.goto(path)
    await expect(page).toHaveURL(/my-keys$/)
  }
})

test('调用记录支持最新优先、翻页、详情和有限正文预览', async ({ page, request }) => {
  await login(page, request)
  const name = `trace-ui-${Date.now()}`
  expect(
    (
      await request.post('/__gateway/plugins', {
        headers: { ...bootstrap, 'Content-Type': 'application/zip' },
        data: fixture(name, 'export default {fetch(r){return new Response(r.body)}}'),
      })
    ).status(),
  ).toBe(201)
  await page.goto(`plugins/${name}`)
  await page.getByRole('link', { name: '查看调用记录', exact: true }).click()
  await expect(page.getByText('暂无调用记录', { exact: true })).toBeVisible()
  for (let i = 0; i < 52; i++) {
    const response = await request.post(`http://127.0.0.1:19081/${name}/call-${i}`, {
      data: i === 51 ? 'x'.repeat(70_000) : `body-${i}`,
    })
    expect(response.status()).toBe(200)
    await response.dispose()
  }
  const listUrl = `/__gateway/plugins/${name}/traces?order=desc&limit=50`
  await expect
    .poll(async () => {
      const r = await request.get(listUrl, { headers: bootstrap })
      return (await r.json()).rows[0]?.data.complete
    })
    .toBe(true)
  await page.getByRole('button', { name: '刷新记录' }).click()
  await expect(page.getByRole('button', { name: /^查看 / })).toHaveCount(50)
  await page
    .getByRole('button', { name: /^查看 / })
    .first()
    .click()
  const dialog = page.getByRole('dialog')
  await expect(
    dialog.getByText(`POST http://127.0.0.1:19081/${name}/call-51`, { exact: true }),
  ).toBeVisible()
  await dialog.getByRole('button', { name: '查看响应正文' }).click()
  await expect(dialog.getByText('（内容已截断）', { exact: true })).toBeVisible()
  expect((await dialog.locator('pre').first().textContent())?.length).toBe(65536)
  await dialog.getByRole('button', { name: 'Close', exact: true }).click()
  await page.getByRole('button', { name: '加载更早记录' }).click()
  await expect(page.getByRole('button', { name: /^查看 / })).toHaveCount(52)
  await expect(page.getByRole('button', { name: '加载更早记录' })).toHaveCount(0)
  await page
    .getByRole('button', { name: /^查看 / })
    .last()
    .click()
  await dialog.getByRole('button', { name: '查看请求正文' }).click()
  await expect(dialog.locator('pre').first()).toHaveText('body-0')
  await dialog.getByRole('button', { name: 'Close', exact: true }).click()
  expect(
    (
      await request.put(`/__gateway/plugins/${name}/config`, {
        headers: bootstrap,
        data: { trace_body: false },
      })
    ).status(),
  ).toBe(200)
  await request.post(`http://127.0.0.1:19081/${name}/hidden`, { data: 'not recorded' })
  await expect
    .poll(async () => {
      const r = await request.get(listUrl, { headers: bootstrap })
      return (await r.json()).rows[0]?.data.body_recorded
    })
    .toBe(false)
  await page.getByRole('button', { name: '刷新记录' }).click()
  await expect(page.getByRole('cell', { name: '未记录', exact: true }).first()).toBeVisible()
  await page
    .getByRole('button', { name: /^查看 / })
    .first()
    .click()
  await expect(dialog.getByText('这次调用未开启正文记录。', { exact: false })).toBeVisible()
  await expect(dialog.getByRole('button', { name: '查看响应正文' })).toHaveCount(0)
  const records = await request.get(listUrl, { headers: bootstrap })
  expect(records.headers()['cache-control']).toBe('no-store')
  const latest = (await records.json()).rows[0]
  const body = await request.get(`/__gateway/plugins/${name}/traces/${latest.request_id}/body`, {
    headers: bootstrap,
  })
  expect(body.headers()['cache-control']).toBe('no-store')
  expect(
    (await request.delete(`/__gateway/plugins/${name}`, { headers: bootstrap })).status(),
  ).toBe(204)
})

test('Core 用量一次上报一行，支持插件隔离、明细翻页和错误恢复', async ({ page, request }) => {
  await login(page, request)
  const names = [`observe-a-${Date.now()}`, `observe-b-${Date.now()}`]
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  for (const [index, name] of names.entries()) {
    const code = `export default {fetch(r,env){console.info('fixture-log-${index}');env.USAGE.record({model:'fixture-model-${index}',input_tokens:${index ? 17 : 3},output_tokens:2});return new Response('ok')}}`
    expect(
      (
        await request.post('/__gateway/plugins', {
          headers: { ...bootstrap, 'Content-Type': 'application/zip' },
          data: fixture(name, code, "[[unsafe.bindings]]\nname='USAGE'\ntype='usage'\n"),
        })
      ).status(),
    ).toBe(201)
    expect((await request.get(`http://127.0.0.1:19081/${name}/call`)).status()).toBe(200)
  }
  // Repeated reports with the same model must stay separate, including across pages.
  for (const [index, name] of names.entries()) {
    for (let i = 0; i < (index ? 50 : 1); i++) {
      expect((await request.get(`http://127.0.0.1:19081/${name}/call-${i}`)).status()).toBe(200)
    }
  }
  await expect
    .poll(
      async () =>
        (
          await (
            await request.get(`/__gateway/plugins/${names[1]}/usage/records?limit=100`, {
              headers: bootstrap,
            })
          ).json()
        ).rows.length,
    )
    .toBe(51)
  await page.getByRole('link', { name: '日志', exact: true }).click()
  await page.getByLabel('插件', { exact: true }).click()
  await page.getByRole('option', { name: names[0], exact: true }).click()
  await expect(page.getByText('fixture-log-0', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('fixture-log-1', { exact: true })).toHaveCount(0)
  await page.getByLabel('插件', { exact: true }).click()
  await page.getByRole('option', { name: names[1], exact: true }).click()
  await expect(page.getByText('fixture-log-1', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('fixture-log-0', { exact: true })).toHaveCount(0)
  await page.goto(`usage?plugin=${names[1]}`)
  await expect(page.getByRole('heading', { name: '用量', exact: true })).toBeVisible()
  const table = page.getByTestId('usage-records')
  await expect(table.locator('tbody tr')).toHaveCount(50)
  await expect(page.getByRole('cell', { name: '17', exact: true }).first()).toBeVisible()
  await expect(page.getByText('合计', { exact: true })).toHaveCount(0)
  await expect(page.getByLabel('分组字段（可选）')).toHaveCount(0)
  await page.getByRole('button', { name: '加载更早用量', exact: true }).click()
  await expect(table.locator('tbody tr')).toHaveCount(51)
  await expect(page.getByRole('button', { name: '加载更早用量', exact: true })).toHaveCount(0)
  const ids = await table.locator('tbody tr td:nth-child(3)').allTextContents()
  expect(new Set(ids).size).toBe(51)
  await page.getByLabel('插件', { exact: true }).click()
  await page.getByRole('option', { name: names[0], exact: true }).click()
  await expect(table.locator('tbody tr')).toHaveCount(2)
  await expect(page.getByRole('cell', { name: '3', exact: true })).toHaveCount(2)
  await expect(page.getByRole('cell', { name: '17', exact: true })).toHaveCount(0)
  await page.route('**/usage/records?**', (route) => route.abort())
  await page.getByRole('button', { name: '查询', exact: true }).click()
  await expect(page.getByRole('alert')).toBeVisible()
  await page.unroute('**/usage/records?**')
  await page.getByRole('button', { name: '查询', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(table.locator('tbody tr')).toHaveCount(2)
  await page.getByLabel('插件', { exact: true }).click()
  await page.getByRole('option', { name: '全部插件', exact: true }).click()
  await expect(page).toHaveURL(/\/usage$/)
  await expect(table.locator('tbody tr')).toHaveCount(50)
  await page.getByRole('button', { name: '加载更早用量', exact: true }).click()
  await expect(table.locator('tbody tr')).toHaveCount(53)
  await expect(table.getByRole('cell', { name: names[0], exact: true })).toHaveCount(2)
  await expect(table.getByRole('cell', { name: names[1], exact: true })).toHaveCount(51)
  expect(new Set(await table.locator('tbody tr td:nth-child(3)').allTextContents()).size).toBe(53)
  await page.reload()
  await expect(page.getByLabel('插件', { exact: true })).toContainText('全部插件')
  await expect(table.locator('tbody tr')).toHaveCount(50)
  await page.goto('logs')
  await expect(page.getByLabel('插件', { exact: true })).toContainText('全部插件')
  await expect(page.locator('tbody tr')).toHaveCount(50)
  await page.getByRole('button', { name: '加载更早日志', exact: true }).click()
  await expect(page.locator('tbody tr')).toHaveCount(53)
  await expect(page.getByRole('cell', { name: names[0], exact: true })).toHaveCount(2)
  await expect(page.getByRole('cell', { name: names[1], exact: true })).toHaveCount(51)
  await page.goto('traces')
  await expect(page.getByLabel('插件', { exact: true })).toContainText('全部插件')
  await expect(page.getByRole('cell', { name: names[1], exact: true }).first()).toBeVisible()
  await page
    .getByRole('button', { name: /^查看 / })
    .first()
    .click()
  const dialog = page.getByRole('dialog')
  await expect(
    dialog.getByText(`GET http://127.0.0.1:19081/${names[1]}/call-49`, { exact: true }),
  ).toBeVisible()
  await dialog.getByRole('button', { name: '查看响应正文' }).click()
  await expect(dialog.locator('pre').first()).toHaveText('ok')
  await expect(dialog.getByRole('alert')).toHaveCount(0)
  await dialog.getByRole('button', { name: 'Close', exact: true }).click()
  for (const path of ['logs', 'traces', 'usage/records']) {
    const response = await request.get(`/__gateway/${path}?order=desc&limit=2`, {
      headers: bootstrap,
    })
    expect(response.status()).toBe(200)
    expect(response.headers()['cache-control']).toBe('no-store')
    expect((await request.get(`/__gateway/${path}`)).status()).toBe(401)
  }
  await page.goto('usage?plugin=no-reports-fixture')
  await expect(page.getByText('暂无用量', { exact: true })).toBeVisible()
  expect(errors).toEqual([])
  for (const name of names)
    await request.delete(`/__gateway/plugins/${name}`, { headers: bootstrap })
})
