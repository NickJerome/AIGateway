import { computed, shallowRef } from 'vue'
import { api, ApiError, getToken, messageOf, setToken, type User } from './api'

const user = shallowRef<User | null>(null)
const notice = shallowRef('')
let restoring: Promise<void> | null = null

async function restore() {
  if (user.value || !getToken()) return
  if (!restoring) {
    restoring = (async () => {
      try {
        user.value = await api<User>('/me')
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 401)) notice.value = messageOf(e)
      }
    })().finally(() => {
      restoring = null
    })
  }
  await restoring
}

async function login(login: string, password: string) {
  const result = await api<{ token: string }>('/auth/login', {
    method: 'POST',
    body: { login, password },
    token: null,
  })
  const current = await api<User>('/me', { token: result.token })
  setToken(result.token)
  user.value = current
  notice.value = ''
}

function clear(message = '') {
  setToken(null)
  user.value = null
  notice.value = message
}

async function logout() {
  try {
    await api('/auth/logout', { method: 'POST' })
  } finally {
    clear()
  }
}

export const session = {
  user,
  notice,
  isAdmin: computed(() => user.value?.administrator === true),
  restore,
  login,
  logout,
  clear,
}
