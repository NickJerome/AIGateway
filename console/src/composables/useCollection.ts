import { onMounted, onScopeDispose, ref, shallowRef } from 'vue'
import { messageOf } from '@/lib/api'

export function useCollection<T>(fetcher: () => Promise<T[]>) {
  const items = shallowRef<T[]>([])
  const loading = ref(true)
  const error = ref('')
  let generation = 0
  async function refresh() {
    const current = ++generation
    loading.value = true
    error.value = ''
    try {
      const result = await fetcher()
      if (generation === current) items.value = result
    } catch (e) {
      if (generation === current) error.value = messageOf(e)
    } finally {
      if (generation === current) loading.value = false
    }
  }
  onMounted(refresh)
  onScopeDispose(() => {
    generation++
  })
  return { items, loading, error, refresh }
}
