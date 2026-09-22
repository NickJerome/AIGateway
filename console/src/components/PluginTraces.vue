<script setup lang="ts">
import { ref, watch } from 'vue'
import { RefreshCw, Activity } from '@lucide/vue'
import { api, apiText, messageOf } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from '@/components/ui/table'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import { ScrollArea, ScrollBar } from '@/components/ui/scroll-area'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'

interface Trace {
  method: string
  url: string
  status?: number
  complete: boolean
  error?: string | null
  duration_ms?: number
  request_bytes?: number
  response_bytes?: number
  body_recorded: boolean
  attempts?: { target: string; status?: number; error?: string | null }[]
}
interface Row {
  id: number
  time: number
  plugin_id: string
  request_id: string
  data: Trace
}
const props = defineProps<{ name?: string }>()
const rows = ref<Row[]>([]),
  more = ref(false),
  loading = ref(false),
  error = ref('')
const selected = ref<Row | null>(null),
  detail = ref<Trace | null>(null),
  detailError = ref('')
const detailLoading = ref(false),
  bodyLoading = ref(false),
  bodyError = ref('')
const preview = ref<{ text: string; truncated: boolean } | null>(null),
  previewSide = ref('')
const open = ref(false)
let generation = 0,
  detailGeneration = 0
const endpoint = () =>
  props.name ? `/plugins/${encodeURIComponent(props.name)}/traces` : '/traces'
const detailEndpoint = (row: Row) =>
  `/plugins/${encodeURIComponent(row.plugin_id)}/traces/${encodeURIComponent(row.request_id)}`
const pathOf = (url: string) => {
  try {
    const u = new URL(url)
    return u.pathname + u.search
  } catch {
    return url
  }
}
const stateOf = (trace: Trace) => (trace.complete ? '已完成' : trace.error ? '已中断' : '处理中')
async function load(older = false) {
  const current = generation
  loading.value = true
  error.value = ''
  try {
    const cursor = older ? `&before_id=${rows.value.at(-1)?.id}` : ''
    const result = await api<{ rows: Row[]; has_more: boolean }>(
      `${endpoint()}?order=desc&limit=50${cursor}`,
    )
    if (current !== generation) return
    rows.value = older ? [...rows.value, ...result.rows] : result.rows
    more.value = result.has_more
  } catch (e) {
    if (current === generation) error.value = messageOf(e)
  } finally {
    if (current === generation) loading.value = false
  }
}
watch(
  () => props.name,
  () => {
    generation++
    detailGeneration++
    rows.value = []
    more.value = false
    open.value = false
    void load()
  },
  { immediate: true },
)
async function inspect(row: Row) {
  const current = ++detailGeneration
  selected.value = row
  detail.value = row.data
  preview.value = null
  bodyError.value = detailError.value = ''
  bodyLoading.value = false
  open.value = detailLoading.value = true
  try {
    const result = await api<Trace>(detailEndpoint(row))
    if (current === detailGeneration) detail.value = result
  } catch (e) {
    if (current === detailGeneration) detailError.value = messageOf(e)
  } finally {
    if (current === detailGeneration) detailLoading.value = false
  }
}
async function loadBody(side: string) {
  if (!selected.value || bodyLoading.value) return
  const current = detailGeneration
  bodyLoading.value = true
  bodyError.value = ''
  preview.value = null
  previewSide.value = side === 'request' ? '请求正文' : '响应正文'
  try {
    const result = await apiText(`${detailEndpoint(selected.value)}/body?side=${side}`)
    if (current === detailGeneration) preview.value = result
  } catch (e) {
    if (current === detailGeneration) bodyError.value = messageOf(e)
  } finally {
    if (current === detailGeneration) bodyLoading.value = false
  }
}
</script>
<template>
  <Card class="shadow-none">
    <CardHeader class="flex-row items-start justify-between gap-3">
      <div class="space-y-2">
        <CardTitle>调用记录</CardTitle>
        <CardDescription
          >按最新时间排列，包含进入插件执行的请求及其上游调用。记录写入可能稍有延迟。</CardDescription
        >
      </div>
      <Button variant="outline" size="sm" :disabled="loading" @click="load()"
        ><RefreshCw :class="{ 'animate-spin': loading }" />刷新记录</Button
      >
    </CardHeader>
    <CardContent class="space-y-4">
      <Alert v-if="error" variant="destructive"
        ><AlertDescription>{{ error }}</AlertDescription></Alert
      >
      <Skeleton v-if="loading && !rows.length" class="h-48" />
      <Table v-else-if="rows.length">
        <TableHeader
          ><TableRow>
            <TableHead>时间</TableHead><TableHead>插件</TableHead><TableHead>请求</TableHead
            ><TableHead>状态</TableHead> <TableHead>耗时</TableHead><TableHead>响应大小</TableHead
            ><TableHead>正文</TableHead><TableHead><span class="sr-only">操作</span></TableHead>
          </TableRow></TableHeader
        >
        <TableBody
          ><TableRow v-for="row in rows" :key="row.id">
            <TableCell class="whitespace-nowrap text-xs">{{
              new Date(row.time).toLocaleString()
            }}</TableCell>
            <TableCell>{{ row.plugin_id }}</TableCell>
            <TableCell
              ><div class="flex items-center gap-2">
                <Badge variant="outline">{{ row.data.method }}</Badge
                ><span class="max-w-64 truncate font-mono text-xs" :title="row.data.url">{{
                  pathOf(row.data.url)
                }}</span>
              </div></TableCell
            >
            <TableCell
              ><Badge
                :variant="
                  row.data.error || (row.data.status ?? 0) >= 400 ? 'destructive' : 'secondary'
                "
                >{{ row.data.status ?? '—' }} · {{ stateOf(row.data) }}</Badge
              ></TableCell
            >
            <TableCell class="whitespace-nowrap">{{
              row.data.duration_ms == null ? '—' : `${row.data.duration_ms} ms`
            }}</TableCell>
            <TableCell class="whitespace-nowrap">{{
              row.data.response_bytes == null ? '—' : `${row.data.response_bytes} B`
            }}</TableCell>
            <TableCell>{{ row.data.body_recorded ? '已开启' : '未记录' }}</TableCell>
            <TableCell
              ><Button
                size="sm"
                variant="ghost"
                :aria-label="`查看 ${row.request_id}`"
                @click="inspect(row)"
                >查看详情</Button
              ></TableCell
            >
          </TableRow></TableBody
        >
      </Table>
      <Empty v-else-if="!error"
        ><EmptyHeader
          ><EmptyMedia variant="icon"><Activity /></EmptyMedia><EmptyTitle>暂无调用记录</EmptyTitle
          ><EmptyDescription
            >调用插件接口后刷新即可查看。鉴权失败等进入插件前被拒绝的请求不在此列表。</EmptyDescription
          ></EmptyHeader
        ></Empty
      >
      <div v-if="more" class="flex justify-center">
        <Button variant="outline" :disabled="loading" @click="load(true)">加载更早记录</Button>
      </div>
    </CardContent>
  </Card>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-3xl">
      <DialogHeader
        ><DialogTitle>调用详情</DialogTitle
        ><DialogDescription class="break-all">{{
          selected?.request_id
        }}</DialogDescription></DialogHeader
      >
      <ScrollArea class="h-[65vh] pr-4">
        <div v-if="detail" class="space-y-5">
          <Alert v-if="detailError" variant="destructive"
            ><AlertDescription>{{ detailError }}</AlertDescription></Alert
          >
          <p v-if="detailLoading" class="text-sm text-muted-foreground">正在更新详情…</p>
          <div class="space-y-2">
            <p class="break-all font-mono text-xs">{{ detail.method }} {{ detail.url }}</p>
            <p>
              {{ detail.status ?? '—' }} · {{ stateOf(detail) }} ·
              {{ detail.duration_ms ?? '—' }} ms
            </p>
            <p v-if="detail.error" class="break-all text-destructive">{{ detail.error }}</p>
          </div>
          <div class="space-y-2">
            <p class="font-medium">上游调用</p>
            <p v-if="!detail.attempts?.length" class="text-sm text-muted-foreground">
              未记录到上游调用。
            </p>
            <div
              v-for="(attempt, index) in detail.attempts"
              :key="index"
              class="space-y-1 rounded-lg border p-3 text-xs"
            >
              <p class="break-all font-mono">{{ attempt.target }}</p>
              <p>状态：{{ attempt.status ?? '—' }}</p>
              <p v-if="attempt.error" class="break-all text-destructive">{{ attempt.error }}</p>
            </div>
          </div>
          <div class="space-y-3">
            <p class="font-medium">正文预览</p>
            <p v-if="!detail.body_recorded" class="text-sm text-muted-foreground">
              这次调用未开启正文记录。可在运行配置中开启，仅影响之后的调用。
            </p>
            <template v-else>
              <div class="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  :disabled="bodyLoading"
                  @click="loadBody('request')"
                  >查看请求正文</Button
                ><Button
                  variant="outline"
                  size="sm"
                  :disabled="bodyLoading"
                  @click="loadBody('response')"
                  >查看响应正文</Button
                >
              </div>
              <p class="text-xs text-muted-foreground">
                按 UTF-8 预览，最多显示 64 KiB；进行中的请求可能尚未记录完整。
              </p>
              <p v-if="bodyLoading" class="text-sm text-muted-foreground">正在加载正文…</p>
              <Alert v-if="bodyError" variant="destructive"
                ><AlertDescription>{{ bodyError }}</AlertDescription></Alert
              >
              <template v-if="preview"
                ><p class="text-sm">
                  {{ previewSide }}<span v-if="preview.truncated">（内容已截断）</span>
                </p>
                <ScrollArea class="h-48 rounded-lg border bg-muted/30">
                  <pre class="p-3 font-mono text-xs">{{ preview.text || '（空正文）' }}</pre>
                  <ScrollBar orientation="horizontal" /></ScrollArea
              ></template>
            </template>
          </div>
          <div class="space-y-2">
            <p class="font-medium">记录元数据</p>
            <ScrollArea class="h-64 rounded-lg border bg-muted/30">
              <pre class="p-3 font-mono text-xs">{{ JSON.stringify(detail, null, 2) }}</pre>
              <ScrollBar orientation="horizontal"
            /></ScrollArea>
          </div>
        </div>
      </ScrollArea>
    </DialogContent>
  </Dialog>
</template>
