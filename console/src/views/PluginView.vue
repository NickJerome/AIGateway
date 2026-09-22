<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from 'vue'
import { useRouter } from 'vue-router'
import { ArrowLeft, Puzzle, Power, RefreshCw, Save, Plus, Trash2, Copy } from '@lucide/vue'
import { toast } from 'vue-sonner'
import PageHeading from '@/components/PageHeading.vue'
import ConfirmAction, { type Confirmation } from '@/components/ConfirmAction.vue'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
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
import { Skeleton } from '@/components/ui/skeleton'
import { api, messageOf } from '@/lib/api'
import { plugins, routePath, pluginStatus, type Plugin, type PluginConfig } from '@/lib/plugins'
const props = defineProps<{ name: string }>()
const router = useRouter()
const plugin = shallowRef<Plugin | null>(null),
  config = shallowRef<PluginConfig | null>(null)
const loading = ref(true),
  error = ref(''),
  tab = ref('overview'),
  formError = ref(''),
  trace = ref('inherit')
const action = ref<Confirmation | null>(null)
interface Variable {
  name: string
  value: string
  type: 'text' | 'json'
  inherited: boolean
}
const variables = ref<Variable[]>([])
const endpoint = computed(() => `/plugins/${encodeURIComponent(props.name)}`)
const effective = computed(() => ({ ...config.value?.defaults, ...config.value?.overrides.vars }))
const secretName = (name: string) => /key|token|secret|password|credential/i.test(name)
async function load() {
  loading.value = true
  error.value = ''
  try {
    const [detail, settings] = await Promise.all([
      api<Plugin>(endpoint.value),
      api<PluginConfig>(`${endpoint.value}/config`),
    ])
    plugin.value = detail
    config.value = settings
    const names = new Set([
      ...Object.keys(settings.defaults),
      ...Object.keys(settings.overrides.vars),
    ])
    if (props.name === 'octopus') names.add('API_KEY')
    variables.value = [...names].sort().map((name) => {
      const value = { ...settings.defaults, ...settings.overrides.vars }[name] ?? ''
      return {
        name,
        type: typeof value === 'string' ? 'text' : 'json',
        value: typeof value === 'string' ? value : JSON.stringify(value, null, 2),
        inherited: !(name in settings.overrides.vars) && name in settings.defaults,
      }
    })
    trace.value =
      settings.overrides.trace_body == null
        ? 'inherit'
        : settings.overrides.trace_body
          ? 'on'
          : 'off'
  } catch (e) {
    error.value = messageOf(e)
  } finally {
    loading.value = false
  }
}
onMounted(load)
async function refresh() {
  await load()
  await plugins.refresh()
}
function toggle() {
  if (!plugin.value) return
  const enabled = !plugin.value.enabled
  action.value = {
    title: `${enabled ? '启用' : '停用'} ${props.name}？`,
    description: enabled ? '插件接口将恢复服务。' : '当前请求会被中断，配置和数据会保留。',
    label: enabled ? '启用插件' : '停用插件',
    destructive: !enabled,
    run: async () => {
      try {
        await api(`${endpoint.value}/enabled`, {
          method: 'PUT',
          body: { enabled },
          timeout: 60_000,
        })
        toast.success(enabled ? '插件已启用' : '插件已停用')
      } finally {
        await refresh()
      }
    },
  }
}
function save() {
  if (!config.value || loading.value) return
  formError.value = ''
  const vars: Record<string, unknown> = Object.create(null)
  const names = new Set<string>()
  try {
    for (const row of variables.value) {
      const name = row.name.trim()
      if (!name || names.has(name)) throw new Error('变量名称不能为空或重复。')
      if (name === 'IDENTITY') throw new Error('IDENTITY 为系统保留字段。')
      names.add(name)
      if (!row.inherited) vars[name] = row.type === 'json' ? JSON.parse(row.value) : row.value
    }
  } catch (e) {
    formError.value = e instanceof SyntaxError ? 'JSON 值格式不正确，请检查变量。' : messageOf(e)
    return
  }
  const body = {
    ...config.value.overrides,
    vars,
    trace_body: trace.value === 'inherit' ? null : trace.value === 'on',
  }
  action.value = {
    title: '保存插件配置？',
    description: '保存后插件会重新加载，正在处理的请求可能中断。',
    label: '保存并应用',
    run: async () => {
      try {
        await api(`${endpoint.value}/config`, { method: 'PUT', body, timeout: 60_000 })
        toast.success('配置已保存')
      } finally {
        await refresh()
      }
    },
  }
}
function uninstall() {
  action.value = {
    title: `卸载 ${props.name}？`,
    description:
      '插件代码和路由将被移除，当前请求会中断。存储数据与运行覆盖配置保留，同名重装后可恢复。',
    label: '卸载插件',
    destructive: true,
    run: async () => {
      await api(endpoint.value, { method: 'DELETE', timeout: 60_000 })
      await plugins.refresh()
      toast.success('插件已卸载')
      await router.replace('/plugins')
    },
  }
}
function resetDefault(index: number) {
  const row = variables.value[index]!
  const value = config.value!.defaults[row.name]
  row.value = typeof value === 'string' ? value : JSON.stringify(value, null, 2)
  row.type = typeof value === 'string' ? 'text' : 'json'
  row.inherited = true
}
async function copy(value: string) {
  try {
    await navigator.clipboard.writeText(value)
    toast.success('已复制')
  } catch {
    toast.error('复制失败，请手动选择并复制。')
  }
}
</script>
<template>
  <div class="space-y-6">
    <Button variant="ghost" size="sm" as-child
      ><RouterLink to="/plugins"><ArrowLeft />插件管理</RouterLink></Button
    >
    <PageHeading
      :title="name"
      description="管理这个插件的接口和运行配置。"
      eyebrow="PLUGIN WORKSPACE"
      ><Button
        variant="outline"
        size="icon"
        aria-label="刷新插件工作台"
        :disabled="loading"
        @click="refresh"
        ><RefreshCw :class="{ 'animate-spin': loading }" /></Button
      ><Button v-if="plugin" variant="outline" :disabled="loading" @click="toggle"
        ><Power />{{ plugin.enabled ? '停用插件' : '启用插件' }}</Button
      ></PageHeading
    >
    <Alert v-if="error" variant="destructive"
      ><AlertDescription
        >{{ error
        }}<Button variant="outline" size="sm" @click="refresh">重试</Button></AlertDescription
      ></Alert
    >
    <Skeleton v-if="loading" class="h-64 w-full rounded-xl" />
    <template v-else-if="plugin && !error">
      <Alert v-if="plugin.error" variant="destructive"
        ><AlertDescription class="break-all">{{ plugin.error }}</AlertDescription></Alert
      >
      <div class="flex flex-wrap gap-2">
        <Button
          v-for="(label, kind) in { traces: '查看调用记录', logs: '查看日志', usage: '查看用量' }"
          :key="kind"
          variant="outline"
          size="sm"
          as-child
        >
          <RouterLink :to="{ path: `/${kind}`, query: { plugin: name } }">{{ label }}</RouterLink>
        </Button>
      </div>
      <Tabs v-model="tab"
        ><TabsList
          ><TabsTrigger value="overview">概览与接口</TabsTrigger
          ><TabsTrigger value="config">运行配置</TabsTrigger></TabsList
        >
        <TabsContent value="overview" class="mt-5 space-y-5">
          <div class="grid gap-5 lg:grid-cols-3">
            <Card class="shadow-none"
              ><CardHeader
                ><CardDescription>运行状态</CardDescription
                ><CardTitle class="flex items-center gap-3"
                  ><Puzzle class="size-5 text-primary" />{{ name
                  }}<Badge
                    variant="outline"
                    :class="
                      plugin.error
                        ? 'status-warning'
                        : plugin.enabled
                          ? 'status-success'
                          : 'status-muted'
                    "
                    >{{ pluginStatus(plugin) }}</Badge
                  ></CardTitle
                ></CardHeader
              ><CardContent class="text-xs text-muted-foreground"
                >{{ plugin.routes.length }} 个接口入口 ·
                {{ plugin.bindings.length }} 个存储绑定</CardContent
              ></Card
            ><Card class="shadow-none lg:col-span-2"
              ><CardHeader
                ><CardTitle>{{ name === 'octopus' ? 'Octopus 模型服务' : '插件信息' }}</CardTitle
                ><CardDescription>{{
                  name === 'octopus'
                    ? '三种原生格式直接转发，支持 JSON 和 SSE。'
                    : '此工作台随插件安装自动生成。'
                }}</CardDescription></CardHeader
              ><CardContent class="space-y-2 text-sm"
                ><p v-if="typeof effective.MODEL === 'string'">
                  模型：<span class="font-mono">{{ effective.MODEL }}</span>
                </p>
                <p v-if="name === 'octopus'" class="text-muted-foreground">
                  上游凭证：{{
                    effective.API_KEY ? '已配置' : '未配置，请到运行配置填写 API_KEY'
                  }}。调用时使用网关个人或服务 Key。
                </p>
                <p class="text-xs text-muted-foreground">
                  入口文件：{{ plugin.main }}
                </p></CardContent
              ></Card
            >
          </div>
          <Card class="gap-0 py-0 shadow-none"
            ><CardHeader class="py-5"
              ><CardTitle>接口入口</CardTitle
              ><CardDescription
                >使用网关 API 地址加下列路径调用。页面地址与 API 端口可能不同。</CardDescription
              ></CardHeader
            ><Table
              ><TableHeader
                ><TableRow
                  ><TableHead class="pl-5">路径</TableHead><TableHead>鉴权</TableHead
                  ><TableHead class="pr-5 text-right">操作</TableHead></TableRow
                ></TableHeader
              ><TableBody
                ><TableRow v-for="route in plugin.routes" :key="routePath(route)"
                  ><TableCell class="py-4 pl-5 font-mono text-xs">{{ routePath(route) }}</TableCell
                  ><TableCell
                    ><Badge variant="outline">{{
                      typeof route === 'string'
                        ? '公开'
                        : ({ key: '网关 Key', user: '登录用户', admin: '管理员', public: '公开' }[
                            route.auth
                          ] ?? route.auth)
                    }}</Badge></TableCell
                  ><TableCell class="pr-5 text-right"
                    ><Button
                      variant="ghost"
                      size="icon-sm"
                      :aria-label="`复制 ${routePath(route)}`"
                      @click="copy(routePath(route))"
                      ><Copy /></Button></TableCell></TableRow></TableBody
            ></Table>
            <p v-if="!plugin.routes.length" class="p-5 text-sm text-muted-foreground">
              此插件没有注册 HTTP 入口。
            </p></Card
          >
          <Card v-if="plugin.bindings.length" class="shadow-none"
            ><CardHeader><CardTitle>存储绑定</CardTitle></CardHeader
            ><CardContent
              ><p
                v-for="binding in plugin.bindings"
                :key="binding.binding"
                class="mb-2 break-all font-mono text-xs"
              >
                {{ binding.binding }} → {{ binding.id }}
              </p></CardContent
            ></Card
          >
          <Card class="border-destructive/20 shadow-none"
            ><CardContent class="flex flex-wrap items-center justify-between gap-4"
              ><div>
                <p class="text-sm font-medium">卸载插件</p>
                <p class="mt-1 text-xs text-muted-foreground">移除代码和路由，保留已存储的数据。</p>
              </div>
              <Button variant="outline" class="text-destructive" @click="uninstall"
                ><Trash2 />卸载插件</Button
              ></CardContent
            ></Card
          >
        </TabsContent>
        <TabsContent value="config" class="mt-5 space-y-5">
          <Card class="shadow-none"
            ><CardHeader
              ><CardTitle>环境变量</CardTitle
              ><CardDescription
                >沿用安装包默认值，或为当前部署设置覆盖值。凭证字段默认隐藏。</CardDescription
              ></CardHeader
            ><CardContent class="space-y-5">
              <Alert v-if="formError" variant="destructive"
                ><AlertDescription>{{ formError }}</AlertDescription></Alert
              >
              <div
                v-for="(row, index) in variables"
                :key="index"
                class="space-y-3 rounded-lg border p-4"
              >
                <div class="flex flex-wrap items-center gap-3">
                  <Input
                    v-model="row.name"
                    :aria-label="`变量名称 ${index + 1}`"
                    class="font-mono sm:max-w-xs"
                    :disabled="!!config && row.name in config.defaults"
                    @update:model-value="row.inherited = false"
                  /><Badge v-if="row.inherited" variant="secondary">安装包默认值</Badge
                  ><Select v-model="row.type" @update:model-value="row.inherited = false"
                    ><SelectTrigger :aria-label="`${row.name} 值类型`" class="w-28"
                      ><SelectValue /></SelectTrigger
                    ><SelectContent
                      ><SelectItem value="text">文本</SelectItem
                      ><SelectItem value="json">JSON</SelectItem></SelectContent
                    ></Select
                  ><Button
                    v-if="config && row.name in config.defaults"
                    variant="ghost"
                    size="sm"
                    @click="resetDefault(index)"
                    >恢复默认</Button
                  ><Button
                    v-else
                    variant="ghost"
                    size="icon-sm"
                    :aria-label="`删除变量 ${row.name || index + 1}`"
                    @click="variables.splice(index, 1)"
                    ><Trash2
                  /></Button>
                </div>
                <Field
                  ><FieldLabel :for="`variable-${index}`">{{ row.name || '变量' }} 的值</FieldLabel
                  ><Input
                    v-if="row.type === 'text'"
                    :id="`variable-${index}`"
                    v-model="row.value"
                    :type="secretName(row.name) ? 'password' : 'text'"
                    autocomplete="off"
                    @update:model-value="row.inherited = false" /><ScrollArea
                    v-else
                    class="max-h-56 [&_[data-slot=scroll-area-viewport]]:max-h-56"
                    ><Textarea
                      :id="`variable-${index}`"
                      v-model="row.value"
                      class="min-h-24 resize-none overflow-hidden font-mono text-xs"
                      @update:model-value="row.inherited = false" /></ScrollArea
                ></Field>
              </div>
              <p v-if="!variables.length" class="text-sm text-muted-foreground">
                尚未设置环境变量。
              </p>
              <Button
                variant="outline"
                @click="variables.push({ name: '', value: '', type: 'text', inherited: false })"
                ><Plus />添加变量</Button
              >
            </CardContent></Card
          >
          <Card class="shadow-none"
            ><CardHeader
              ><CardTitle>请求记录</CardTitle
              ><CardDescription>控制是否保存该插件的请求与响应正文。</CardDescription></CardHeader
            ><CardContent
              ><Field
                ><FieldLabel for="trace-body">正文记录</FieldLabel
                ><Select v-model="trace"
                  ><SelectTrigger id="trace-body" class="w-full sm:w-60"
                    ><SelectValue /></SelectTrigger
                  ><SelectContent
                    ><SelectItem value="inherit">沿用网关设置</SelectItem
                    ><SelectItem value="on">开启</SelectItem
                    ><SelectItem value="off">关闭</SelectItem></SelectContent
                  ></Select
                ><FieldDescription>开启后会保存调用内容，用于排查问题。</FieldDescription></Field
              ></CardContent
            ></Card
          >
          <div class="flex items-center justify-end gap-3">
            <Button variant="outline" @click="load">放弃修改</Button
            ><Button @click="save"><Save />保存配置</Button>
          </div>
        </TabsContent>
      </Tabs>
    </template>
    <ConfirmAction :action="action" @close="action = null" />
  </div>
</template>
