export interface User {
  id: string
  login: string
  enabled: boolean
  administrator: boolean
  created_at: number
}

export interface ApiKey {
  id: string
  user_id: string | null
  name: string
  revoked: boolean
  expires_at: number | null
  created_at: number
}

export interface CreatedKey extends ApiKey {
  key: string
}

const SESSION_KEY = 'aigateway.session'
export const getToken = () => sessionStorage.getItem(SESSION_KEY)
export function setToken(token: string | null) {
  if (token) sessionStorage.setItem(SESSION_KEY, token)
  else sessionStorage.removeItem(SESSION_KEY)
}

const messages: Record<string, string> = {
  'invalid or expired credentials': '账号或密码不正确，或登录已过期。',
  'administrator authorization required': '此操作需要管理员权限。',
  'login name already exists': '这个用户名已存在，请使用其他名称。',
  'invalid login name': '用户名只能包含英文字母、数字和 . _ - @ +。',
  'password must contain 12–1024 bytes': '密码至少需要 12 个字节，最多 1,024 个字节。',
  'cannot disable or demote the last administrator': '请至少保留一位可用的管理员。',
  'active user not found': '用户不存在或已被停用，请刷新后重试。',
  'user not found': '用户不存在，请刷新后重试。',
  'key not found': '找不到此 Key，或你没有管理权限。',
  'invalid key name or expiry (Unix seconds)': '请检查 Key 名称和有效期。',
  'login capacity exhausted; retry later': '当前登录请求较多，请稍后重试。',
  'identity service unavailable': '身份服务暂时不可用，请稍后重试。',
  'invalid request fields': '提交内容不正确，请检查表单。',
}

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request(
  path: string,
  options: {
    method?: string
    body?: unknown
    token?: string | null
    timeout?: number
  } = {},
): Promise<Response> {
  const token = options.token === undefined ? getToken() : options.token
  let response: Response
  try {
    response = await fetch(`/__gateway${path}`, {
      method: options.method ?? 'GET',
      headers: {
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
        ...(options.body === undefined
          ? {}
          : {
              'Content-Type': options.body instanceof Blob ? 'application/zip' : 'application/json',
            }),
      },
      body:
        options.body instanceof Blob
          ? options.body
          : options.body === undefined
            ? undefined
            : JSON.stringify(options.body),
      signal: AbortSignal.timeout(options.timeout ?? 15_000),
      cache: 'no-store',
      credentials: 'omit',
    })
  } catch {
    throw new ApiError(0, '无法连接网关，请检查服务是否运行后重试。')
  }
  if (!response.ok) {
    const data = await response.json().catch(() => ({}))
    if (response.status === 401 && options.token === undefined && token && token === getToken()) {
      setToken(null)
      window.dispatchEvent(new Event('gateway:session-expired'))
    }
    throw new ApiError(
      response.status,
      messages[data.message] ??
        (data.code === 'plugin_management'
          ? `插件操作失败：${data.message}`
          : `请求未完成（${response.status}），请稍后重试。`),
    )
  }
  return response
}

export async function api<T>(
  path: string,
  options: Parameters<typeof request>[1] = {},
): Promise<T> {
  const response = await request(path, options)
  if (response.status === 204) return undefined as T
  const data = await response.json().catch(() => {
    throw new ApiError(502, '网关返回了无效数据，请检查连接配置。')
  })
  return data as T
}

// Preview only: never buffer an unbounded inference response in the browser.
export async function apiText(path: string): Promise<{ text: string; truncated: boolean }> {
  const response = await request(path)
  const reader = response.body?.getReader()
  if (!reader) return { text: '', truncated: false }
  const decoder = new TextDecoder()
  const limit = 64 * 1024
  let size = 0,
    text = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) return { text: text + decoder.decode(), truncated: false }
      const remaining = limit - size
      text += decoder.decode(value.subarray(0, remaining), { stream: true })
      size += value.byteLength
      if (size > limit) return { text: text + decoder.decode(), truncated: true }
    }
  } finally {
    await reader.cancel().catch(() => {})
  }
}

export function messageOf(error: unknown) {
  return error instanceof Error ? error.message : '操作失败，请稍后重试。'
}
