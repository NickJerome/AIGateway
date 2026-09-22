import { test, expect, type Page, type APIRequestContext } from '@playwright/test'
const PASSWORD = 'console-test-password-2026'
const headers = { Authorization: 'Bearer console-isolated-test-bootstrap' }
let counter = 0
let browserErrors: string[] = []
test.beforeEach(({ page }) => {
  browserErrors = []
  page.on('pageerror', (error) => browserErrors.push(error.message))
})
test.afterEach(() => {
  expect(browserErrors).toEqual([])
})
async function createUser(request: APIRequestContext, admin = true) {
  const login = `test-${Date.now()}-${counter++}`
  const res = await request.post('/__gateway/users', {
    headers,
    data: { login, password: PASSWORD, administrator: admin },
  })
  expect(res.status()).toBe(201)
  return (await res.json()) as { id: string; login: string }
}
async function login(page: Page, name: string) {
  await page.goto('login')
  await page.getByLabel('用户名', { exact: true }).fill(name)
  await page.getByLabel('密码', { exact: true }).fill(PASSWORD)
  await page.getByRole('button', { name: '登录控制台' }).click()
  await expect(page.getByTestId('page-scroll-area')).toBeVisible()
}
async function createKey(page: Page, name: string) {
  await page.getByRole('button', { name: '创建 Key', exact: true }).first().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('名称', { exact: true }).fill(name)
  await dialog.getByRole('button', { name: '创建 Key', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Key 已创建' })).toBeVisible()
  const secret = await page.getByLabel('完整 Key').inputValue()
  expect(secret.length).toBeGreaterThan(30)
  await page.getByRole('button', { name: '我已保存' }).click()
  await expect(page.getByLabel('完整 Key')).toHaveCount(0)
  expect(
    await page.evaluate(
      (value) => JSON.stringify({ ...sessionStorage, ...localStorage }).includes(value),
      secret,
    ),
  ).toBe(false)
  return page.getByRole('row').filter({ hasText: name })
}

test('首次设置、登录与用户管理', async ({ page }) => {
  const loginName = `bootstrap-${Date.now()}`
  await page.goto('login')
  await expect(page.getByRole('heading', { name: '欢迎回来' })).toBeVisible()
  await page.screenshot({ path: 'artifacts/login.png' })
  await page.getByRole('tab', { name: '首次设置' }).click()
  await page.getByLabel('平台管理令牌').fill('console-isolated-test-bootstrap')
  await page.getByLabel('用户名', { exact: true }).fill(loginName)
  await page.getByLabel('密码', { exact: true }).fill(PASSWORD)
  await page.getByRole('button', { name: '创建并登录' }).click()
  await expect(page.getByRole('heading', { name: '用户管理', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '创建用户', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('用户名', { exact: true }).fill(`member-${Date.now()}`)
  await dialog.getByLabel('初始密码').fill(PASSWORD)
  await dialog.getByRole('button', { name: '创建用户', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  const row = page.getByRole('row').filter({ hasText: 'member-' }).first()
  await row.getByRole('button').click()
  await page.getByRole('menuitem', { name: '停用用户' }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '停用用户' }).click()
  await expect(row.getByText('已停用', { exact: true })).toBeVisible()
  await page.screenshot({ path: 'artifacts/users-desktop.png' })
})

test('个人与服务 Key 创建、一次性展示及撤销', async ({ page, request }) => {
  const user = await createUser(request)
  await login(page, user.login)
  await page.getByRole('link', { name: 'Key 管理', exact: true }).click()
  const service = await createKey(page, 'production-service')
  await expect(service.getByText('服务 Key', { exact: true })).toBeVisible()
  await service.getByRole('button').click()
  await page.getByRole('menuitem', { name: '撤销 Key' }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '撤销 Key' }).click()
  await expect(service.getByText('已撤销', { exact: true })).toBeVisible()
  await page.getByRole('link', { name: '我的 Key', exact: true }).click()
  const personal = await createKey(page, 'my-editor')
  await expect(personal.getByText('有效', { exact: true })).toBeVisible()
  await page.reload()
  await expect(page.getByLabel('完整 Key')).toHaveCount(0)
  await expect(page.getByRole('row').filter({ hasText: 'my-editor' })).toBeVisible()
  await page.getByRole('button', { name: '切换深色模式' }).click()
  await page.screenshot({ path: 'artifacts/keys-dark.png' })
})

test('普通用户只能管理自己；管理员停用后会话退出', async ({ page, request }) => {
  const user = await createUser(request, false)
  await login(page, user.login)
  await expect(page.getByRole('link', { name: '用户管理', exact: true })).toHaveCount(0)
  await page.goto('users')
  await expect(page).toHaveURL(/my-keys$/)
  await createKey(page, 'member-key')
  await expect(page.getByRole('row').filter({ hasText: 'production-service' })).toHaveCount(0)
  expect(
    (
      await request.put(`/__gateway/users/${user.id}`, { headers, data: { enabled: false } })
    ).status(),
  ).toBe(200)
  await page.getByRole('button', { name: '刷新 Key' }).click()
  await expect(page).toHaveURL(/login$/)
  await expect(page.getByRole('alert')).toContainText('登录已过期')
})

test('手机布局：导航、表格和页面使用 ScrollArea', async ({ page, request }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  const user = await createUser(request)
  await login(page, user.login)
  await page.getByRole('button', { name: '切换导航' }).click()
  await page.getByRole('link', { name: '我的 Key', exact: true }).click()
  await createKey(page, 'mobile-key')
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  const viewport = page
    .getByTestId('page-scroll-area')
    .locator('[data-slot="scroll-area-viewport"]')
    .first()
  await viewport.evaluate((el) => {
    el.scrollTop = 500
  })
  expect(await viewport.evaluate((el) => el.scrollTop)).toBeGreaterThan(0)
  const table = page.locator('[data-slot="table-container"] [data-slot="scroll-area-viewport"]')
  expect(await table.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true)
  await table.evaluate((el) => {
    el.scrollLeft = 500
  })
  expect(await table.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0)
  await page.screenshot({ path: 'artifacts/mobile.png' })
  await page.setViewportSize({ width: 844, height: 390 })
  await page.getByRole('button', { name: '创建 Key', exact: true }).first().click()
  const bounds = await page.getByRole('dialog').boundingBox()
  expect(bounds!.y).toBeGreaterThanOrEqual(0)
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(390)
  await page.getByRole('button', { name: '取消', exact: true }).click()
})

test('错误登录与请求失败可恢复', async ({ page, request }) => {
  const user = await createUser(request)
  await page.goto('login')
  await page.getByLabel('用户名', { exact: true }).fill(user.login)
  await page.getByLabel('密码', { exact: true }).fill('wrong-password')
  await page.getByRole('button', { name: '登录控制台' }).click()
  await expect(page.getByRole('alert')).toBeVisible()
  await login(page, user.login)
  await page.route('**/__gateway/users', (route) =>
    route.fulfill({
      status: 503,
      contentType: 'application/json',
      body: JSON.stringify({ message: 'identity service unavailable' }),
    }),
  )
  await page.getByRole('button', { name: '刷新用户' }).click()
  await expect(page.getByRole('alert')).toContainText('暂时不可用')
  await page.unroute('**/__gateway/users')
  await page.getByRole('button', { name: '重试', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('角色和密码修改，以及账号页和退出登录', async ({ page, request }) => {
  const admin = await createUser(request)
  const member = await createUser(request, false)
  await login(page, admin.login)
  await page.getByLabel('搜索用户').fill(member.login)
  const row = page.getByRole('row').filter({ hasText: member.login })
  await row.getByRole('button').click()
  await page.getByRole('menuitem', { name: '修改角色' }).click()
  await page.getByRole('dialog').getByLabel('角色', { exact: true }).click()
  await page.getByRole('option', { name: '管理员', exact: true }).click()
  await page.getByRole('button', { name: '保存修改' }).click()
  await expect(row.getByText('管理员', { exact: true })).toBeVisible()
  await row.getByRole('button').click()
  await page.getByRole('menuitem', { name: '重置密码' }).click()
  const newPassword = 'changed-console-password-2026'
  await page.getByLabel('新密码').fill(newPassword)
  await page.getByRole('button', { name: '保存修改' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  expect(
    (
      await request.post('/__gateway/auth/login', {
        data: { login: member.login, password: PASSWORD },
      })
    ).status(),
  ).toBe(401)
  expect(
    (
      await request.post('/__gateway/auth/login', {
        data: { login: member.login, password: newPassword },
      })
    ).status(),
  ).toBe(200)
  await page.getByRole('link', { name: '我的账号', exact: true }).click()
  await expect(page.getByRole('heading', { name: '我的账号', exact: true })).toBeVisible()
  await expect(page.getByText(admin.id, { exact: true })).toBeVisible()
  await page.getByRole('button', { name: new RegExp(`${admin.login}.*系统管理员`) }).click()
  await page.getByRole('menuitem', { name: '退出登录' }).click()
  await expect(page).toHaveURL(/login$/)
  expect(await page.evaluate(() => sessionStorage.getItem('aigateway.session'))).toBeNull()
})

test('管理员指定 Key 归属与有效期；表格分页和筛选', async ({ page, request }) => {
  const admin = await createUser(request)
  const member = await createUser(request, false)
  await login(page, admin.login)
  await page.getByRole('link', { name: 'Key 管理', exact: true }).click()
  await page.getByRole('button', { name: '创建 Key', exact: true }).first().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('名称', { exact: true }).fill('assigned-key')
  await dialog.getByLabel('归属', { exact: true }).click()
  await page.getByRole('option', { name: `${member.login} · 个人 Key`, exact: true }).click()
  await dialog.getByLabel('有效期', { exact: true }).click()
  await page.getByRole('option', { name: '7 天', exact: true }).click()
  await dialog.getByRole('button', { name: '创建 Key', exact: true }).click()
  await page.getByRole('button', { name: '我已保存' }).click()
  await expect(page.getByRole('row').filter({ hasText: 'assigned-key' })).toContainText(
    member.login,
  )
  const res = await request.get('/__gateway/keys', { headers })
  const key = (await res.json()).find((key: { name: string }) => key.name === 'assigned-key')
  expect(key.user_id).toBe(member.id)
  expect(key.expires_at - Math.floor(Date.now() / 1000)).toBeGreaterThan(6 * 86400)
  for (let i = 0; i < 11; i++) {
    const response = await request.post('/__gateway/keys', {
      headers,
      data: { name: `page-fixture-${i}` },
    })
    expect(response.status()).toBe(201)
  }
  await page.getByRole('button', { name: '刷新 Key' }).click()
  await page.getByLabel('搜索 Key').fill('page-fixture-')
  await expect(page.getByRole('row')).toHaveCount(11)
  await page.getByRole('button', { name: '下一页' }).click()
  await expect(page.getByRole('row')).toHaveCount(2)
  await page.getByLabel('搜索 Key').fill('no-match-at-all')
  await expect(page.getByText('没有匹配的 Key', { exact: true })).toBeVisible()
})
