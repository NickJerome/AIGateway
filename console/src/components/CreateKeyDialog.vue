<script setup lang="ts">
import { ref, watch } from 'vue'
import { LoaderCircle } from '@lucide/vue'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, messageOf, type User, type CreatedKey } from '@/lib/api'
const props = defineProps<{ open: boolean; scope: 'all' | 'mine'; users: User[] }>()
const emit = defineEmits<{ close: []; created: [key: CreatedKey] }>()
const name = ref(''),
  owner = ref('service'),
  expiry = ref('30'),
  busy = ref(false),
  error = ref('')
watch(
  () => props.open,
  () => {
    name.value = ''
    owner.value = 'service'
    expiry.value = '30'
    error.value = ''
  },
)
async function create() {
  if (!name.value.trim()) {
    error.value = '请输入 Key 名称。'
    return
  }
  busy.value = true
  error.value = ''
  try {
    const key = await api<CreatedKey>(props.scope === 'mine' ? '/me/keys' : '/keys', {
      method: 'POST',
      body: {
        name: name.value.trim(),
        expires_at:
          expiry.value === 'never'
            ? null
            : Math.floor(Date.now() / 1000) + Number(expiry.value) * 86400,
        ...(props.scope === 'all'
          ? { user_id: owner.value === 'service' ? null : owner.value }
          : {}),
      },
    })
    emit('close')
    emit('created', key)
  } catch (e) {
    error.value = messageOf(e)
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
    ><DialogContent
      class="sm:max-w-md"
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
        ><DialogTitle>创建 Key</DialogTitle
        ><DialogDescription>为一个工具或应用创建独立的访问凭证。</DialogDescription></DialogHeader
      >
      <form @submit.prevent="create">
        <ScrollArea
          class="max-h-[calc(100dvh-16rem)] [&_[data-slot=scroll-area-viewport]]:max-h-[calc(100dvh-16rem)]"
          ><div class="space-y-5 px-0.5 py-2">
            <Alert v-if="error" variant="destructive"
              ><AlertDescription>{{ error }}</AlertDescription></Alert
            >
            <Field
              ><FieldLabel for="key-name">名称</FieldLabel
              ><Input
                id="key-name"
                v-model="name"
                :disabled="busy"
                placeholder="例如：我的开发工具"
                maxlength="128"
                required
            /></Field>
            <Field v-if="scope === 'all'"
              ><FieldLabel for="key-owner">归属</FieldLabel
              ><Select v-model="owner" :disabled="busy"
                ><SelectTrigger id="key-owner" class="w-full"><SelectValue /></SelectTrigger
                ><SelectContent
                  ><SelectItem value="service">平台 · 服务 Key</SelectItem
                  ><SelectItem
                    v-for="user in users.filter((u) => u.enabled)"
                    :key="user.id"
                    :value="user.id"
                    >{{ user.login }} · 个人 Key</SelectItem
                  ></SelectContent
                ></Select
              ><FieldDescription
                >服务 Key 属于平台，适用于后台服务；个人 Key 关联指定用户。</FieldDescription
              ></Field
            >
            <Field
              ><FieldLabel for="key-expiry">有效期</FieldLabel
              ><Select v-model="expiry" :disabled="busy"
                ><SelectTrigger id="key-expiry" class="w-full"><SelectValue /></SelectTrigger
                ><SelectContent
                  ><SelectItem value="7">7 天</SelectItem><SelectItem value="30">30 天</SelectItem
                  ><SelectItem value="90">90 天</SelectItem
                  ><SelectItem value="never">永久有效</SelectItem></SelectContent
                ></Select
              ></Field
            >
          </div></ScrollArea
        ><DialogFooter class="mt-6"
          ><Button type="button" variant="outline" :disabled="busy" @click="emit('close')"
            >取消</Button
          ><Button type="submit" :disabled="busy"
            ><LoaderCircle v-if="busy" class="size-4 animate-spin" />创建 Key</Button
          ></DialogFooter
        >
      </form>
    </DialogContent></Dialog
  >
</template>
