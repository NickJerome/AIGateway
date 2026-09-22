<script setup lang="ts">
import { ref, watch } from 'vue'
import { LoaderCircle } from '@lucide/vue'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from '@/components/ui/alert-dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { messageOf } from '@/lib/api'

export interface Confirmation {
  title: string
  description: string
  label: string
  destructive?: boolean
  run: () => Promise<void>
}
const props = defineProps<{ action: Confirmation | null }>()
const emit = defineEmits<{ close: [] }>()
const busy = ref(false)
const error = ref('')
watch(
  () => props.action,
  () => {
    error.value = ''
  },
)
async function confirm() {
  if (!props.action || busy.value) return
  busy.value = true
  try {
    await props.action.run()
    emit('close')
  } catch (e) {
    error.value = messageOf(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AlertDialog
    :open="!!action"
    @update:open="
      (value) => {
        if (!value && !busy) emit('close')
      }
    "
  >
    <AlertDialogContent
      @escape-key-down="
        (event) => {
          if (busy) event.preventDefault()
        }
      "
    >
      <AlertDialogHeader>
        <AlertDialogTitle>{{ action?.title }}</AlertDialogTitle>
        <AlertDialogDescription>{{ action?.description }}</AlertDialogDescription>
      </AlertDialogHeader>
      <Alert v-if="error" variant="destructive"
        ><AlertDescription>{{ error }}</AlertDescription></Alert
      >
      <AlertDialogFooter>
        <AlertDialogCancel :disabled="busy">取消</AlertDialogCancel>
        <AlertDialogAction
          :disabled="busy"
          :class="action?.destructive ? 'bg-destructive text-white hover:bg-destructive/90' : ''"
          @click.prevent="confirm"
        >
          <LoaderCircle v-if="busy" class="size-4 animate-spin" />{{ action?.label }}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
