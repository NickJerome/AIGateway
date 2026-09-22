import { ref, shallowRef, watch } from 'vue'
import { api, messageOf } from './api'
import { session } from './session'
export interface Plugin {
  name: string
  state: string
  enabled: boolean
  error: string | null
  main: string
  source_hash: string | null
  routes: (string | { pattern: string; auth: string })[]
  bindings: { binding: string; id: string }[]
}
export interface PluginConfig {
  defaults: Record<string, unknown>
  overrides: {
    vars: Record<string, unknown>
    bindings: Record<string, string>
    trace_body: boolean | null
  }
}
const items = shallowRef<Plugin[]>([])
const loading = ref(false),
  error = ref('')
let generation = 0
async function refresh() {
  if (!session.isAdmin.value) return
  const current = ++generation
  loading.value = true
  error.value = ''
  try {
    const result = await api<Plugin[]>('/plugins')
    if (current === generation) items.value = result
  } catch (e) {
    if (current === generation) error.value = messageOf(e)
  } finally {
    if (current === generation) loading.value = false
  }
}
watch(
  () => session.user.value?.id,
  () => {
    generation++
    items.value = []
    error.value = ''
    loading.value = false
  },
)
export const plugins = { items, loading, error, refresh }
export const pluginPath = (name: string) => `/plugins/${encodeURIComponent(name)}`
export const pluginStatus = (p: Plugin) =>
  p.error || p.state === 'failed' ? '异常' : p.enabled ? '运行中' : '已停用'
export const routePath = (route: Plugin['routes'][number]) =>
  typeof route === 'string' ? route : route.pattern
