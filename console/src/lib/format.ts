import type { ApiKey } from './api'

export function dateTime(value: number | null) {
  return value == null
    ? '永久有效'
    : new Intl.DateTimeFormat('zh-CN', {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        hour12: false,
      }).format(new Date(value * 1000))
}

export function keyState(key: ApiKey, now = Date.now()) {
  if (key.revoked) return { value: 'revoked', label: '已撤销', class: 'status-muted' }
  if (key.expires_at !== null && key.expires_at * 1000 <= now)
    return { value: 'expired', label: '已过期', class: 'status-warning' }
  return { value: 'active', label: '有效', class: 'status-success' }
}

export function passwordError(password: string) {
  const length = new TextEncoder().encode(password).length
  return length < 12 || length > 1024
    ? '密码需为 12–1,024 字节，建议使用至少 12 位字母、数字和符号。'
    : ''
}
