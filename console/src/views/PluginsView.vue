<script setup lang="ts">
import { computed, ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { Blocks, Puzzle, Upload, RefreshCw, Search, ArrowUpRight, Power } from '@lucide/vue'
import { toast } from 'vue-sonner'
import PageHeading from '@/components/PageHeading.vue'
import InstallPluginDialog from '@/components/InstallPluginDialog.vue'
import ConfirmAction, { type Confirmation } from '@/components/ConfirmAction.vue'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { InputGroup, InputGroupInput, InputGroupAddon } from '@/components/ui/input-group'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { api } from '@/lib/api'
import { plugins, pluginPath, pluginStatus, type Plugin } from '@/lib/plugins'
const router = useRouter()
const { items, loading, error } = plugins
const search = ref(''),
  upload = ref(false),
  action = ref<Confirmation | null>(null)
const filtered = computed(() =>
  items.value.filter((p) => p.name.toLowerCase().includes(search.value.toLowerCase())),
)
onMounted(() => {
  void plugins.refresh()
})
function toggle(plugin: Plugin) {
  const enabled = !plugin.enabled
  action.value = {
    title: `${enabled ? '启用' : '停用'} ${plugin.name}？`,
    description: enabled
      ? '启用后，插件注册的接口将恢复服务。'
      : '停用会中断当前请求，插件配置和数据会保留。',
    label: enabled ? '启用插件' : '停用插件',
    destructive: !enabled,
    run: async () => {
      try {
        await api(`/plugins/${encodeURIComponent(plugin.name)}/enabled`, {
          method: 'PUT',
          body: { enabled },
          timeout: 60_000,
        })
        toast.success(enabled ? '插件已启用' : '插件已停用')
      } finally {
        await plugins.refresh()
      }
    },
  }
}
</script>
<template>
  <div class="space-y-7">
    <PageHeading
      title="插件管理"
      description="安装插件、管理运行状态，进入各个插件的工作台。"
      eyebrow="WORKSPACE"
      ><Button
        variant="outline"
        size="icon"
        aria-label="刷新插件"
        :disabled="loading"
        @click="plugins.refresh"
        ><RefreshCw :class="{ 'animate-spin': loading }" /></Button
      ><Button @click="upload = true"><Upload />安装插件</Button></PageHeading
    >
    <div class="flex flex-wrap items-center justify-between gap-4">
      <InputGroup class="w-full sm:max-w-xs"
        ><InputGroupAddon><Search /></InputGroupAddon
        ><InputGroupInput v-model="search" placeholder="搜索插件…" aria-label="搜索插件"
      /></InputGroup>
      <p class="text-xs text-muted-foreground">
        {{ items.length }} 个已安装 ·
        {{ items.filter((p) => p.enabled && !p.error).length }} 个运行中
      </p>
    </div>
    <Alert v-if="error" variant="destructive"
      ><AlertDescription
        >{{ error
        }}<Button variant="outline" size="sm" @click="plugins.refresh"
          >重试</Button
        ></AlertDescription
      ></Alert
    >
    <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
      <template v-if="loading && !items.length"
        ><Skeleton v-for="i in 3" :key="i" class="h-64 rounded-xl"
      /></template>
      <Card v-for="plugin in filtered" :key="plugin.name" class="shadow-none">
        <CardHeader
          ><div class="mb-3 flex items-center justify-between">
            <div
              class="flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary"
            >
              <Puzzle class="size-5" />
            </div>
            <Badge
              variant="outline"
              :class="
                plugin.error ? 'status-warning' : plugin.enabled ? 'status-success' : 'status-muted'
              "
              >{{ pluginStatus(plugin) }}</Badge
            >
          </div>
          <CardTitle class="break-all">{{ plugin.name }}</CardTitle
          ><CardDescription
            >{{ plugin.routes.length }} 个入口 ·
            {{ plugin.bindings.length }} 个存储绑定</CardDescription
          ></CardHeader
        >
        <CardContent class="flex-1"
          ><p v-if="plugin.error" class="line-clamp-3 break-all text-xs text-destructive">
            {{ plugin.error }}
          </p>
          <p v-else class="text-sm leading-6 text-muted-foreground">
            {{
              plugin.name === 'octopus'
                ? '通过 Octopus 调用 Chat Completions、Responses 和 Messages，支持流式返回。'
                : '在工作台查看接口、调整运行配置和检查日志。'
            }}
          </p></CardContent
        >
        <CardFooter class="flex justify-between gap-3"
          ><Button variant="ghost" size="sm" @click="toggle(plugin)"
            ><Power />{{ plugin.enabled ? '停用' : '启用' }}</Button
          ><Button variant="outline" size="sm" as-child
            ><RouterLink :to="pluginPath(plugin.name)"
              >进入工作台<ArrowUpRight /></RouterLink></Button
        ></CardFooter>
      </Card>
    </div>
    <Empty
      v-if="!loading && !error && !filtered.length"
      class="min-h-72 rounded-xl border border-dashed"
      ><EmptyHeader
        ><EmptyMedia variant="icon"><Blocks /></EmptyMedia
        ><EmptyTitle>{{ items.length ? '没有匹配的插件' : '安装第一个插件' }}</EmptyTitle
        ><EmptyDescription
          >安装后，每个插件都会出现在左侧快捷入口中。</EmptyDescription
        ></EmptyHeader
      ><Button v-if="!items.length" @click="upload = true"><Upload />安装插件</Button></Empty
    >
    <InstallPluginDialog
      :open="upload"
      @close="upload = false"
      @installed="(plugin) => router.push(pluginPath(plugin.name))"
    /><ConfirmAction :action="action" @close="action = null" />
  </div>
</template>
