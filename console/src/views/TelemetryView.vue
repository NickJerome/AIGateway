<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RefreshCw } from '@lucide/vue'
import PageHeading from '@/components/PageHeading.vue'
import PluginTraces from '@/components/PluginTraces.vue'
import { plugins } from '@/lib/plugins'
import { api, messageOf } from '@/lib/api'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select'
import { Field, FieldLabel } from '@/components/ui/field'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from '@/components/ui/table'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription } from '@/components/ui/empty'

const props = defineProps<{ kind: 'traces' | 'logs' | 'usage' }>()
const route = useRoute(),
  router = useRouter()
const selected = ref(typeof route.query.plugin === 'string' ? route.query.plugin : '__all__')
const period = ref('7'),
  loading = ref(false),
  error = ref('')
interface Log {
  id: number
  time: number
  plugin_id: string
  request_id: string
  data: { level: string; message: string }
}
interface Usage {
  id: number
  time: number
  plugin_id: string
  request_id: string
  data: Record<string, string | number | boolean | null>
}
const logs = ref<Log[]>([]),
  usage = ref<Usage[]>([]),
  more = ref(false)
const title = computed(() => ({ traces: '调用记录', logs: '日志', usage: '用量' })[props.kind])
const description = computed(
  () =>
    ({
      traces: 'Core 自动采集的插件请求与上游调用。',
      logs: '查看各插件通过 console 打印的日志。',
      usage: '每次 usage 上报单独显示一条，最新记录在前。',
    })[props.kind],
)
const fields = computed(() => {
  const keys = new Set(usage.value.flatMap((row) => Object.keys(row.data)))
  const primary = ['model', 'protocol', 'input_tokens', 'output_tokens', 'total_tokens']
  return [...primary.filter((key) => keys.delete(key)), ...[...keys].sort()]
})
const labels: Record<string, string> = {
  model: '模型',
  protocol: '协议',
  input_tokens: '输入 tokens',
  output_tokens: '输出 tokens',
  total_tokens: '总 tokens',
}
const display = (value: unknown) =>
  value == null ? '—' : typeof value === 'boolean' ? (value ? '是' : '否') : String(value)
let since = 0
let generation = 0
onMounted(() => plugins.refresh())
watch(
  () => route.query.plugin,
  (name) => {
    selected.value = typeof name === 'string' ? name : '__all__'
  },
)
watch(selected, (plugin) => {
  void router.replace({
    query: { ...route.query, plugin: plugin === '__all__' ? undefined : plugin },
  })
})
watch(
  [selected, period, () => props.kind],
  () => {
    generation++
    logs.value = []
    usage.value = []
    more.value = false
    error.value = ''
    if (selected.value && props.kind !== 'traces') void load()
  },
  { immediate: true },
)
async function load(older = false) {
  if (!selected.value) return
  const current = ++generation
  loading.value = true
  error.value = ''
  try {
    const base =
      selected.value === '__all__'
        ? `/${props.kind}`
        : `/plugins/${encodeURIComponent(selected.value)}/${props.kind}`
    if (props.kind === 'logs') {
      const result = await api<{ rows: Log[]; has_more: boolean }>(
        `${base}?order=desc&limit=50${older ? `&before_id=${logs.value.at(-1)?.id}` : ''}`,
      )
      if (current !== generation) return
      logs.value = older ? [...logs.value, ...result.rows] : result.rows
      more.value = result.has_more
    } else {
      if (!older) since = period.value === 'all' ? 0 : Date.now() - Number(period.value) * 86400000
      const cursor = older ? `&before_id=${usage.value.at(-1)?.id}` : ''
      const result = await api<{ rows: Usage[]; has_more: boolean }>(
        `${base}/records?order=desc&limit=50&since=${since}${cursor}`,
      )
      if (current !== generation) return
      usage.value = older ? [...usage.value, ...result.rows] : result.rows
      more.value = result.has_more
    }
  } catch (e) {
    if (current === generation) error.value = messageOf(e)
  } finally {
    if (current === generation) loading.value = false
  }
}
</script>
<template>
  <div class="space-y-6">
    <PageHeading :title="title" :description="description" eyebrow="GATEWAY" />
    <Alert v-if="plugins.error.value" variant="destructive"
      ><AlertDescription
        >{{ plugins.error.value
        }}<Button variant="outline" size="sm" @click="plugins.refresh"
          >重试</Button
        ></AlertDescription
      ></Alert
    >
    <div class="flex flex-wrap items-end gap-4">
      <Field class="w-full sm:w-64"
        ><FieldLabel for="telemetry-plugin">插件</FieldLabel
        ><Select v-model="selected"
          ><SelectTrigger id="telemetry-plugin"
            ><SelectValue placeholder="选择插件" /></SelectTrigger
          ><SelectContent>
            <SelectItem value="__all__">全部插件</SelectItem>
            <SelectItem
              v-if="selected !== '__all__' && !plugins.items.value.some((p) => p.name === selected)"
              :value="selected"
              >{{ selected }}（历史记录）</SelectItem
            >
            <SelectItem
              v-for="plugin in plugins.items.value"
              :key="plugin.name"
              :value="plugin.name"
              >{{ plugin.name }}</SelectItem
            >
          </SelectContent></Select
        ></Field
      >
      <template v-if="kind === 'usage'">
        <Field class="w-full sm:w-40"
          ><FieldLabel for="usage-period">时间范围</FieldLabel
          ><Select v-model="period"
            ><SelectTrigger id="usage-period"><SelectValue /></SelectTrigger
            ><SelectContent
              ><SelectItem value="1">最近 24 小时</SelectItem
              ><SelectItem value="7">最近 7 天</SelectItem
              ><SelectItem value="30">最近 30 天</SelectItem
              ><SelectItem value="all">全部保留数据</SelectItem></SelectContent
            ></Select
          ></Field
        >
      </template>
      <Button
        v-if="kind !== 'traces'"
        variant="outline"
        :disabled="loading || !selected"
        @click="load()"
        ><RefreshCw :class="{ 'animate-spin': loading }" />查询</Button
      >
    </div>
    <Empty v-if="!selected && !plugins.loading.value"
      ><EmptyHeader
        ><EmptyTitle>暂无插件</EmptyTitle
        ><EmptyDescription>安装并调用插件后，可在这里查看记录。</EmptyDescription></EmptyHeader
      ></Empty
    >
    <PluginTraces
      v-else-if="kind === 'traces' && selected"
      :key="selected"
      :name="selected === '__all__' ? undefined : selected"
    />
    <template v-else-if="selected">
      <Alert v-if="error" variant="destructive"
        ><AlertDescription>{{ error }}</AlertDescription></Alert
      >
      <Skeleton v-if="loading && !logs.length && !usage.length" class="h-48" />
      <Card v-else-if="kind === 'logs'" class="shadow-none"
        ><CardHeader
          ><CardTitle>Console 日志</CardTitle
          ><CardDescription>最新记录在前；只展示插件实际打印的内容。</CardDescription></CardHeader
        ><CardContent class="space-y-4">
          <Table v-if="logs.length"
            ><TableHeader
              ><TableRow
                ><TableHead>时间</TableHead><TableHead>插件</TableHead><TableHead>级别</TableHead
                ><TableHead>内容</TableHead><TableHead>请求 ID</TableHead></TableRow
              ></TableHeader
            ><TableBody
              ><TableRow v-for="row in logs" :key="row.id">
                <TableCell class="whitespace-nowrap text-xs">{{
                  new Date(row.time).toLocaleString()
                }}</TableCell
                ><TableCell>{{ row.plugin_id }}</TableCell
                ><TableCell
                  ><Badge :variant="row.data.level === 'error' ? 'destructive' : 'secondary'">{{
                    row.data.level
                  }}</Badge></TableCell
                >
                <TableCell
                  ><ScrollArea
                    class="max-h-48 min-w-64 max-w-xl [&_[data-slot=scroll-area-viewport]]:max-h-48"
                  >
                    <pre class="whitespace-pre-wrap break-all font-mono text-xs">{{
                      row.data.message
                    }}</pre>
                  </ScrollArea></TableCell
                ><TableCell class="font-mono text-xs">{{ row.request_id }}</TableCell>
              </TableRow></TableBody
            ></Table
          >
          <Empty v-else-if="!error"
            ><EmptyHeader
              ><EmptyTitle>暂无日志</EmptyTitle
              ><EmptyDescription>所选范围内没有 console 日志。</EmptyDescription></EmptyHeader
            ></Empty
          >
          <Button v-if="more" variant="outline" :disabled="loading" @click="load(true)"
            >加载更早日志</Button
          >
        </CardContent></Card
      >
      <Card v-else class="shadow-none"
        ><CardHeader
          ><CardTitle>用量明细</CardTitle>
          <CardDescription
            >每行是一条原始上报记录，保留各次调用的字段和值。</CardDescription
          > </CardHeader
        ><CardContent class="space-y-4">
          <Table v-if="usage.length" data-testid="usage-records">
            <TableHeader
              ><TableRow>
                <TableHead>时间</TableHead><TableHead>插件</TableHead><TableHead>请求 ID</TableHead>
                <TableHead v-for="field in fields" :key="field">{{
                  labels[field] ?? field
                }}</TableHead>
              </TableRow></TableHeader
            >
            <TableBody
              ><TableRow v-for="row in usage" :key="row.id">
                <TableCell class="whitespace-nowrap text-xs">{{
                  new Date(row.time).toLocaleString()
                }}</TableCell>
                <TableCell>{{ row.plugin_id }}</TableCell>
                <TableCell class="font-mono text-xs">{{ row.request_id }}</TableCell>
                <TableCell v-for="field in fields" :key="field"
                  ><span class="block max-w-64 truncate" :title="display(row.data[field])">{{
                    display(row.data[field])
                  }}</span></TableCell
                >
              </TableRow></TableBody
            >
          </Table>
          <Empty v-else-if="!error"
            ><EmptyHeader
              ><EmptyTitle>暂无用量</EmptyTitle
              ><EmptyDescription
                >所选时间内没有 usage 上报。调用记录本身不会自动生成用量。</EmptyDescription
              ></EmptyHeader
            ></Empty
          >
          <Button v-if="more" variant="outline" :disabled="loading" @click="load(true)"
            >加载更早用量</Button
          >
        </CardContent></Card
      >
    </template>
  </div>
</template>
