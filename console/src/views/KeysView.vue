<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { useNow, useIntervalFn } from '@vueuse/core'
import { KeyRound, CircleCheck, Server, Plus, RefreshCw, Search, MoreHorizontal } from '@lucide/vue'
import { toast } from 'vue-sonner'
import PageHeading from '@/components/PageHeading.vue'
import CreateKeyDialog from '@/components/CreateKeyDialog.vue'
import SecretDialog from '@/components/SecretDialog.vue'
import ConfirmAction, { type Confirmation } from '@/components/ConfirmAction.vue'
import TableFooter from '@/components/TableFooter.vue'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from '@/components/ui/table'
import { InputGroup, InputGroupInput, InputGroupAddon } from '@/components/ui/input-group'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, type ApiKey, type CreatedKey, type User } from '@/lib/api'
import { dateTime, keyState } from '@/lib/format'
import { useCollection } from '@/composables/useCollection'
const props = defineProps<{ scope: 'all' | 'mine' }>()
const users = shallowRef<User[]>([])
const { items, loading, error, refresh } = useCollection<ApiKey>(async () => {
  const [keys, owners] = await Promise.all([
    api<ApiKey[]>(props.scope === 'mine' ? '/me/keys' : '/keys'),
    props.scope === 'all' ? api<User[]>('/users') : Promise.resolve([]),
  ])
  users.value = owners
  return keys
})
const owners = computed(() => new Map(users.value.map((user) => [user.id, user])))
const now = useNow({ scheduler: (update) => useIntervalFn(update, 1000) })
function stateOf(key: ApiKey) {
  const state = keyState(key, now.value.getTime())
  return state.value === 'active' && key.user_id && owners.value.get(key.user_id)?.enabled === false
    ? { value: 'disabled', label: '用户停用', class: 'status-warning' }
    : state
}
const search = ref(''),
  filter = ref('all'),
  kind = ref('all'),
  page = ref(1),
  open = ref(false)
const secret = shallowRef<CreatedKey | null>(null),
  action = ref<Confirmation | null>(null)
const ownerOf = (key: ApiKey) =>
  key.user_id ? (owners.value.get(key.user_id)?.login ?? key.user_id) : '平台'
const filtered = computed(() =>
  items.value.filter(
    (k) =>
      (k.name + k.id + ownerOf(k)).toLowerCase().includes(search.value.toLowerCase()) &&
      (filter.value === 'all' || stateOf(k).value === filter.value) &&
      (kind.value === 'all' || (kind.value === 'service' ? !k.user_id : !!k.user_id)),
  ),
)
const rows = computed(() => filtered.value.slice((page.value - 1) * 10, page.value * 10))
watch([search, filter, kind], () => {
  page.value = 1
})
watch(
  () => filtered.value.length,
  (n) => {
    page.value = Math.max(1, Math.min(page.value, Math.ceil(n / 10)))
  },
)
const stats = computed(() => [
  {
    label: '全部 Key',
    value: items.value.length,
    icon: KeyRound,
    hint: '包含已过期和已撤销的记录',
  },
  {
    label: '有效 Key',
    value: items.value.filter((k) => stateOf(k).value === 'active').length,
    icon: CircleCheck,
    hint: '当前可用于访问网关',
  },
  props.scope === 'all'
    ? {
        label: '服务 Key',
        value: items.value.filter((k) => !k.user_id).length,
        icon: Server,
        hint: '属于平台的应用访问凭证',
      }
    : {
        label: '已撤销 Key',
        value: items.value.filter((k) => k.revoked).length,
        icon: Server,
        hint: '已永久停止访问的凭证',
      },
])
function revoke(key: ApiKey) {
  action.value = {
    title: `撤销「${key.name}」？`,
    description: '使用此 Key 的工具将无法继续访问。撤销后不可恢复，需要重新创建 Key。',
    label: '撤销 Key',
    destructive: true,
    run: async () => {
      await api(`${props.scope === 'mine' ? '/me' : ''}/keys/${key.id}`, { method: 'DELETE' })
      toast.success('Key 已撤销')
      await refresh()
    },
  }
}
function created(key: CreatedKey) {
  secret.value = key
  void refresh()
}
</script>
<template>
  <div class="space-y-7">
    <PageHeading
      :title="scope === 'mine' ? '我的 Key' : 'Key 管理'"
      :description="
        scope === 'mine'
          ? '为你的工具创建独立凭证，随时查看和撤销。'
          : '统一管理个人与服务的访问凭证。'
      "
      :eyebrow="scope === 'mine' ? 'PERSONAL' : 'WORKSPACE'"
      ><Button
        variant="outline"
        size="icon"
        aria-label="刷新 Key"
        :disabled="loading"
        @click="refresh"
        ><RefreshCw :class="{ 'animate-spin': loading }" /></Button
      ><Button :disabled="loading || !!error" @click="open = true"
        ><Plus />创建 Key</Button
      ></PageHeading
    >
    <div class="grid gap-4 sm:grid-cols-3">
      <Card v-for="stat in stats" :key="stat.label" class="py-0 shadow-none"
        ><CardContent class="p-5!"
          ><div class="flex items-center justify-between text-sm text-muted-foreground">
            <span>{{ stat.label }}</span
            ><component :is="stat.icon" class="size-4" />
          </div>
          <Skeleton v-if="loading" class="mt-4 h-9 w-16" />
          <p v-else class="mt-3 text-3xl font-semibold tracking-tight">
            {{ error ? '—' : stat.value }}
          </p>
          <p class="mt-2 text-xs text-muted-foreground">{{ stat.hint }}</p></CardContent
        ></Card
      >
    </div>
    <Alert v-if="error" variant="destructive"
      ><AlertDescription class="flex items-center justify-between gap-4"
        >{{ error
        }}<Button variant="outline" size="sm" @click="refresh">重试</Button></AlertDescription
      ></Alert
    >
    <Card class="gap-0 overflow-hidden py-0 shadow-none">
      <div class="flex flex-wrap items-center justify-between gap-3 border-b px-5 py-4">
        <div>
          <h2 class="text-sm font-semibold">访问凭证</h2>
          <p class="mt-1 text-xs text-muted-foreground">完整 Key 仅在创建时显示一次。</p>
        </div>
        <Tabs v-if="scope === 'all'" v-model="kind"
          ><TabsList
            ><TabsTrigger value="all">全部</TabsTrigger
            ><TabsTrigger value="personal">个人 Key</TabsTrigger
            ><TabsTrigger value="service">服务 Key</TabsTrigger></TabsList
          ></Tabs
        >
      </div>
      <div class="flex flex-wrap gap-3 p-5">
        <InputGroup class="w-full sm:max-w-xs"
          ><InputGroupAddon><Search /></InputGroupAddon
          ><InputGroupInput
            v-model="search"
            aria-label="搜索 Key"
            placeholder="搜索名称、ID 或归属…" /></InputGroup
        ><Select v-model="filter"
          ><SelectTrigger aria-label="筛选 Key 状态" class="w-[140px]"
            ><SelectValue /></SelectTrigger
          ><SelectContent
            ><SelectItem value="all">全部状态</SelectItem
            ><SelectItem value="active">有效</SelectItem
            ><SelectItem value="expired">已过期</SelectItem
            ><SelectItem value="revoked">已撤销</SelectItem
            ><SelectItem v-if="scope === 'all'" value="disabled"
              >用户停用</SelectItem
            ></SelectContent
          ></Select
        >
      </div>
      <Table
        ><TableHeader
          ><TableRow class="bg-muted/35 hover:bg-muted/35"
            ><TableHead class="pl-5">名称 / ID</TableHead
            ><TableHead v-if="scope === 'all'">归属</TableHead><TableHead>状态</TableHead
            ><TableHead>有效期至</TableHead><TableHead>创建时间</TableHead
            ><TableHead class="w-16 pr-5 text-right">操作</TableHead></TableRow
          ></TableHeader
        ><TableBody>
          <template v-if="loading"
            ><TableRow v-for="i in 5" :key="i"
              ><TableCell v-for="j in scope === 'all' ? 6 : 5" :key="j" class="px-5 py-5"
                ><Skeleton class="h-5 w-full min-w-12" /></TableCell></TableRow
          ></template>
          <TableRow v-for="key in loading ? [] : rows" :key="key.id"
            ><TableCell class="py-4 pl-5"
              ><div class="flex items-center gap-3">
                <div
                  class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground"
                >
                  <KeyRound class="size-4" />
                </div>
                <div>
                  <p class="max-w-[240px] truncate font-medium" :title="key.name">{{ key.name }}</p>
                  <p
                    class="mt-1 max-w-[210px] truncate font-mono text-[11px] text-muted-foreground"
                    :title="key.id"
                  >
                    {{ key.id }}
                  </p>
                </div>
              </div></TableCell
            ><TableCell v-if="scope === 'all'"
              ><p class="max-w-[160px] truncate text-sm" :title="ownerOf(key)">
                {{ ownerOf(key) }}
              </p>
              <p class="mt-1 text-[11px] text-muted-foreground">
                {{ key.user_id ? '个人 Key' : '服务 Key' }}
              </p></TableCell
            ><TableCell
              ><Badge variant="outline" :class="stateOf(key).class"
                ><span class="size-1.5 rounded-full bg-current" />{{ stateOf(key).label }}</Badge
              ></TableCell
            ><TableCell class="text-xs text-muted-foreground">{{
              dateTime(key.expires_at)
            }}</TableCell
            ><TableCell class="text-xs text-muted-foreground">{{
              dateTime(key.created_at)
            }}</TableCell
            ><TableCell class="pr-5 text-right"
              ><DropdownMenu
                ><DropdownMenuTrigger as-child
                  ><Button
                    variant="ghost"
                    size="icon-sm"
                    :disabled="key.revoked"
                    :aria-label="`管理 ${key.name}`"
                    ><MoreHorizontal /></Button></DropdownMenuTrigger
                ><DropdownMenuContent align="end"
                  ><DropdownMenuItem class="text-destructive" @select="revoke(key)"
                    >撤销 Key</DropdownMenuItem
                  ></DropdownMenuContent
                ></DropdownMenu
              ></TableCell
            ></TableRow
          >
        </TableBody></Table
      >
      <Empty v-if="!loading && !error && !rows.length" class="py-16"
        ><EmptyHeader
          ><EmptyMedia variant="icon"><KeyRound /></EmptyMedia
          ><EmptyTitle>{{ items.length ? '没有匹配的 Key' : '还没有访问凭证' }}</EmptyTitle
          ><EmptyDescription>{{
            items.length
              ? '试试调整搜索内容或筛选条件。'
              : '为你的第一个工具创建 Key，开始使用网关。'
          }}</EmptyDescription></EmptyHeader
        ><Button v-if="!items.length" variant="outline" @click="open = true"
          ><Plus />创建 Key</Button
        ></Empty
      >
      <TableFooter v-model="page" :total="filtered.length" :page-size="10" />
    </Card>
    <p v-if="items.length >= 1000" class="text-xs text-muted-foreground">
      当前接口最多返回 1,000 个 Key，以上为已加载的数据。
    </p>
    <CreateKeyDialog
      :open="open"
      :scope="scope"
      :users="users"
      @close="open = false"
      @created="created"
    /><SecretDialog :value="secret" @close="secret = null" /><ConfirmAction
      :action="action"
      @close="action = null"
    />
  </div>
</template>
