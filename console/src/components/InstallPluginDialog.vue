<script setup lang="ts">
import { ref, watch } from 'vue'
import { LoaderCircle, Upload } from '@lucide/vue'
import { toast } from 'vue-sonner'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, messageOf } from '@/lib/api'
import { plugins, type Plugin } from '@/lib/plugins'
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: []; installed: [plugin: Plugin] }>()
const file = ref<File | null>(null),
  busy = ref(false),
  error = ref('')
watch(
  () => props.open,
  () => {
    file.value = null
    error.value = ''
  },
)
function select(event: Event) {
  file.value = (event.target as HTMLInputElement).files?.[0] ?? null
  error.value = ''
}
async function install() {
  if (!file.value || busy.value) return
  if (!file.value.name.toLowerCase().endsWith('.zip')) {
    error.value = '请选择 ZIP 插件包。'
    return
  }
  busy.value = true
  error.value = ''
  try {
    const result = await api<Plugin>('/plugins', {
      method: 'POST',
      body: file.value,
      timeout: 120_000,
    })
    await plugins.refresh()
    toast.success(`插件 ${result.name} 已安装`)
    emit('close')
    emit('installed', result)
  } catch (e) {
    error.value = messageOf(e)
    await plugins.refresh()
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog
    :open="open"
    @update:open="
      (value) => {
        if (!value && !busy) emit('close')
      }
    "
  >
    <DialogContent
      :show-close-button="!busy"
      @interact-outside="
        (e) => {
          if (busy) e.preventDefault()
        }
      "
      @escape-key-down="
        (e) => {
          if (busy) e.preventDefault()
        }
      "
    >
      <DialogHeader
        ><DialogTitle>安装插件</DialogTitle
        ><DialogDescription>上传 ZIP 后，网关会编译并启用插件。</DialogDescription></DialogHeader
      >
      <Alert v-if="error" variant="destructive"
        ><AlertDescription class="break-all">{{ error }}</AlertDescription></Alert
      >
      <Field
        ><FieldLabel for="plugin-file">插件安装包</FieldLabel
        ><Input
          id="plugin-file"
          type="file"
          accept=".zip,application/zip"
          :disabled="busy"
          @change="select"
        /><FieldDescription>ZIP 根目录需要包含 wrangler.toml 和入口代码。</FieldDescription></Field
      >
      <Alert
        ><AlertDescription
          >同名插件将被替换，现有配置保留。替换会中断该插件正在处理的请求。</AlertDescription
        ></Alert
      >
      <DialogFooter
        ><Button variant="outline" :disabled="busy" @click="emit('close')">取消</Button
        ><Button :disabled="!file || busy" @click="install"
          ><LoaderCircle v-if="busy" class="animate-spin" /><Upload v-else />{{
            busy ? '正在安装…' : '上传并安装'
          }}</Button
        ></DialogFooter
      >
    </DialogContent>
  </Dialog>
</template>
