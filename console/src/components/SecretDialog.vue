<script setup lang="ts">
import { Copy, Check, KeyRound } from '@lucide/vue'
import { ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import type { CreatedKey } from '@/lib/api'
const props = defineProps<{ value: CreatedKey | null }>()
const emit = defineEmits<{ close: [] }>()
const copied = ref(false)
watch(
  () => props.value,
  () => {
    copied.value = false
  },
)
async function copy() {
  if (!props.value) return
  try {
    await navigator.clipboard.writeText(props.value.key)
    copied.value = true
    toast.success('Key 已复制')
  } catch {
    ;(document.getElementById('created-secret') as HTMLInputElement | null)?.select()
    toast.error('无法自动复制，请手动复制已选中的 Key。')
  }
}
</script>
<template>
  <Dialog :open="!!value"
    ><DialogContent
      class="sm:max-w-lg"
      :show-close-button="false"
      @interact-outside.prevent
      @escape-key-down.prevent
    >
      <DialogHeader
        ><div
          class="mb-2 flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary"
        >
          <KeyRound class="size-5" />
        </div>
        <DialogTitle>Key 已创建</DialogTitle
        ><DialogDescription
          >请保存「{{ value?.name }}」的完整 Key，关闭后无法再次查看。</DialogDescription
        ></DialogHeader
      >
      <div class="flex gap-2">
        <Input
          id="created-secret"
          :model-value="value?.key"
          aria-label="完整 Key"
          readonly
          class="font-mono text-xs"
          autocomplete="off"
        /><Button variant="outline" size="icon" aria-label="复制 Key" @click="copy"
          ><Check v-if="copied" /><Copy v-else
        /></Button>
      </div>
      <Alert
        ><AlertDescription
          >把它当作密码保管。若不慎泄露，请撤销并重新创建。</AlertDescription
        ></Alert
      >
      <DialogFooter><Button @click="emit('close')">我已保存</Button></DialogFooter>
    </DialogContent></Dialog
  >
</template>
