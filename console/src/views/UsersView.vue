<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Users, UserCheck, ShieldCheck, Plus, RefreshCw, Search, MoreHorizontal } from '@lucide/vue'
import { toast } from 'vue-sonner'
import PageHeading from '@/components/PageHeading.vue'
import UserDialog from '@/components/UserDialog.vue'
import ConfirmAction, { type Confirmation } from '@/components/ConfirmAction.vue'
import TableFooter from '@/components/TableFooter.vue'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
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
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, type User } from '@/lib/api'
import { session } from '@/lib/session'
import { dateTime } from '@/lib/format'
import { useCollection } from '@/composables/useCollection'
const { items, loading, error, refresh } = useCollection<User>(() => api('/users'))
const search = ref(''),
  role = ref('all'),
  state = ref('all'),
  page = ref(1)
const open = ref(false),
  mode = ref<'create' | 'role' | 'password'>('create'),
  selected = ref<User>()
const action = ref<Confirmation | null>(null)
const filtered = computed(() =>
  items.value.filter(
    (u) =>
      (u.login + u.id).toLowerCase().includes(search.value.toLowerCase()) &&
      (role.value === 'all' || u.administrator === (role.value === 'admin')) &&
      (state.value === 'all' || u.enabled === (state.value === 'enabled')),
  ),
)
const rows = computed(() => filtered.value.slice((page.value - 1) * 10, page.value * 10))
watch([search, role, state], () => {
  page.value = 1
})
watch(
  () => filtered.value.length,
  (n) => {
    page.value = Math.max(1, Math.min(page.value, Math.ceil(n / 10)))
  },
)
const stats = computed(() => [
  { label: '全部用户', value: items.value.length, icon: Users, hint: '工作空间内的全部账号' },
  {
    label: '启用中的用户',
    value: items.value.filter((u) => u.enabled).length,
    icon: UserCheck,
    hint: '可以登录并使用个人 Key',
  },
  {
    label: '管理员',
    value: items.value.filter((u) => u.administrator).length,
    icon: ShieldCheck,
    hint: '拥有用户与 Key 管理权限',
  },
])
function edit(next: typeof mode.value, user?: User) {
  selected.value = user
  mode.value = next
  open.value = true
}
function toggle(user: User) {
  action.value = {
    title: `${user.enabled ? '停用' : '启用'} ${user.login}？`,
    description: user.enabled
      ? '该用户将退出登录，其个人 Key 也会停止访问。重新启用后，未撤销且未过期的 Key 可以继续使用。'
      : '该用户将恢复登录和个人 Key 的使用权限。',
    label: user.enabled ? '停用用户' : '启用用户',
    destructive: user.enabled,
    run: async () => {
      await api(`/users/${user.id}`, { method: 'PUT', body: { enabled: !user.enabled } })
      toast.success(user.enabled ? '用户已停用' : '用户已启用')
      if (user.id === session.user.value?.id)
        window.dispatchEvent(new Event('gateway:session-expired'))
      else await refresh()
    },
  }
}
</script>
<template>
  <div class="space-y-7">
    <PageHeading
      title="用户管理"
      description="管理团队成员、账号状态和管理员权限。"
      eyebrow="WORKSPACE"
      ><Button
        variant="outline"
        size="icon"
        aria-label="刷新用户"
        :disabled="loading"
        @click="refresh"
        ><RefreshCw :class="{ 'animate-spin': loading }" /></Button
      ><Button @click="edit('create')"><Plus />创建用户</Button></PageHeading
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
          <h2 class="text-sm font-semibold">团队成员</h2>
          <p class="mt-1 text-xs text-muted-foreground">在这里管理每一个账号的访问资格。</p>
        </div>
        <Badge variant="secondary">{{ items.length }} 位用户</Badge>
      </div>
      <div class="flex flex-wrap gap-3 p-5">
        <InputGroup class="w-full sm:max-w-xs"
          ><InputGroupAddon><Search /></InputGroupAddon
          ><InputGroupInput
            v-model="search"
            aria-label="搜索用户"
            placeholder="搜索用户名或 ID…" /></InputGroup
        ><Select v-model="role"
          ><SelectTrigger aria-label="筛选角色" class="w-[130px]"><SelectValue /></SelectTrigger
          ><SelectContent
            ><SelectItem value="all">全部角色</SelectItem
            ><SelectItem value="admin">管理员</SelectItem
            ><SelectItem value="member">普通用户</SelectItem></SelectContent
          ></Select
        ><Select v-model="state"
          ><SelectTrigger aria-label="筛选状态" class="w-[130px]"><SelectValue /></SelectTrigger
          ><SelectContent
            ><SelectItem value="all">全部状态</SelectItem
            ><SelectItem value="enabled">已启用</SelectItem
            ><SelectItem value="disabled">已停用</SelectItem></SelectContent
          ></Select
        >
      </div>
      <Table
        ><TableHeader
          ><TableRow class="bg-muted/35 hover:bg-muted/35"
            ><TableHead class="pl-5">用户</TableHead><TableHead>角色</TableHead
            ><TableHead>状态</TableHead><TableHead>创建时间</TableHead
            ><TableHead class="w-16 pr-5 text-right">操作</TableHead></TableRow
          ></TableHeader
        ><TableBody>
          <template v-if="loading"
            ><TableRow v-for="i in 5" :key="i"
              ><TableCell v-for="j in 5" :key="j" class="px-5 py-5"
                ><Skeleton class="h-5 w-full min-w-12" /></TableCell></TableRow
          ></template>
          <TableRow v-for="user in loading ? [] : rows" :key="user.id"
            ><TableCell class="py-4 pl-5"
              ><div class="flex items-center gap-3">
                <Avatar class="size-9 rounded-xl"
                  ><AvatarFallback
                    class="rounded-xl bg-primary/8 text-xs font-medium text-primary"
                    >{{ user.login.slice(0, 2).toUpperCase() }}</AvatarFallback
                  ></Avatar
                >
                <div>
                  <p class="flex items-center gap-2 font-medium">
                    {{ user.login
                    }}<Badge
                      v-if="user.id === session.user.value?.id"
                      variant="outline"
                      class="text-[10px]"
                      >你</Badge
                    >
                  </p>
                  <p
                    class="mt-1 max-w-[210px] truncate font-mono text-[11px] text-muted-foreground"
                    :title="user.id"
                  >
                    {{ user.id }}
                  </p>
                </div>
              </div></TableCell
            ><TableCell
              ><Badge :variant="user.administrator ? 'secondary' : 'outline'"
                ><ShieldCheck v-if="user.administrator" class="size-3" />{{
                  user.administrator ? '管理员' : '普通用户'
                }}</Badge
              ></TableCell
            ><TableCell
              ><Badge variant="outline" :class="user.enabled ? 'status-success' : 'status-muted'"
                ><span class="size-1.5 rounded-full bg-current" />{{
                  user.enabled ? '已启用' : '已停用'
                }}</Badge
              ></TableCell
            ><TableCell class="text-xs text-muted-foreground">{{
              dateTime(user.created_at)
            }}</TableCell
            ><TableCell class="pr-5 text-right"
              ><DropdownMenu
                ><DropdownMenuTrigger as-child
                  ><Button variant="ghost" size="icon-sm" :aria-label="`管理 ${user.login}`"
                    ><MoreHorizontal /></Button></DropdownMenuTrigger
                ><DropdownMenuContent align="end"
                  ><DropdownMenuItem @select="edit('role', user)">修改角色</DropdownMenuItem
                  ><DropdownMenuItem @select="edit('password', user)">重置密码</DropdownMenuItem
                  ><DropdownMenuSeparator /><DropdownMenuItem
                    :class="user.enabled ? 'text-destructive' : ''"
                    @select="toggle(user)"
                    >{{ user.enabled ? '停用用户' : '启用用户' }}</DropdownMenuItem
                  ></DropdownMenuContent
                ></DropdownMenu
              ></TableCell
            ></TableRow
          >
        </TableBody></Table
      >
      <Empty v-if="!loading && !error && !rows.length" class="py-16"
        ><EmptyHeader
          ><EmptyMedia variant="icon"><Users /></EmptyMedia
          ><EmptyTitle>{{ items.length ? '没有匹配的用户' : '还没有团队成员' }}</EmptyTitle
          ><EmptyDescription>{{
            items.length ? '试试调整搜索内容或筛选条件。' : '创建第一个用户，开始协作。'
          }}</EmptyDescription></EmptyHeader
        ></Empty
      >
      <TableFooter v-model="page" :total="filtered.length" :page-size="10" />
    </Card>
    <p v-if="items.length >= 1000" class="text-xs text-muted-foreground">
      当前接口最多返回 1,000 位用户，以上为已加载的数据。
    </p>
    <UserDialog
      :open="open"
      :mode="mode"
      :user="selected"
      @close="open = false"
      @saved="refresh"
    /><ConfirmAction :action="action" @close="action = null" />
  </div>
</template>
