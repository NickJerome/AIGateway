<script setup lang="ts">
import { ref, watch } from 'vue'
import { LoaderCircle } from '@lucide/vue'
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
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, messageOf, type User } from '@/lib/api'
import { passwordError } from '@/lib/format'
import { session } from '@/lib/session'
const props = defineProps<{ open: boolean; user?: User; mode: 'create' | 'role' | 'password' }>()
const emit = defineEmits<{ close: []; saved: [] }>()
const login = ref(''),
  password = ref(''),
  role = ref('member'),
  error = ref(''),
  busy = ref(false)
watch(
  () => props.open,
  () => {
    login.value = props.user?.login ?? ''
    password.value = ''
    role.value = props.user?.administrator ? 'admin' : 'member'
    error.value = ''
  },
)
async function save() {
  error.value = ''
  if (props.mode !== 'role') {
    error.value = passwordError(password.value)
    if (error.value) return
  }
  if (props.mode === 'role' && (role.value === 'admin') === props.user?.administrator) {
    emit('close')
    return
  }
  busy.value = true
  try {
    await api(props.mode === 'create' ? '/users' : `/users/${props.user!.id}`, {
      method: props.mode === 'create' ? 'POST' : 'PUT',
      body:
        props.mode === 'create'
          ? {
              login: login.value.trim(),
              password: password.value,
              administrator: role.value === 'admin',
            }
          : props.mode === 'password'
            ? { password: password.value }
            : { administrator: role.value === 'admin' },
    })
    password.value = ''
    toast.success(props.mode === 'create' ? '用户已创建' : '用户信息已更新')
    if (props.user?.id === session.user.value?.id)
      window.dispatchEvent(new Event('gateway:session-expired'))
    emit('saved')
    emit('close')
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
  >
    <DialogContent
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
        ><DialogTitle>{{
          mode === 'create' ? '创建用户' : mode === 'role' ? '修改用户角色' : '重置密码'
        }}</DialogTitle
        ><DialogDescription>{{
          mode === 'create'
            ? '创建后，将用户名和密码交给团队成员。'
            : `正在管理 ${user?.login}，修改后该用户需要重新登录。`
        }}</DialogDescription></DialogHeader
      >
      <form @submit.prevent="save">
        <ScrollArea
          class="max-h-[calc(100dvh-16rem)] [&_[data-slot=scroll-area-viewport]]:max-h-[calc(100dvh-16rem)]"
          ><div class="space-y-5 px-0.5 py-2">
            <Alert v-if="error" variant="destructive"
              ><AlertDescription>{{ error }}</AlertDescription></Alert
            >
            <Field v-if="mode === 'create'"
              ><FieldLabel for="user-login">用户名</FieldLabel
              ><Input
                id="user-login"
                v-model="login"
                :disabled="busy"
                maxlength="128"
                required
                autocomplete="off"
                placeholder="例如 zhangsan"
              /><FieldDescription>支持英文字母、数字及 . _ - @ +</FieldDescription></Field
            >
            <Field v-if="mode !== 'role'"
              ><FieldLabel for="user-password">{{
                mode === 'create' ? '初始密码' : '新密码'
              }}</FieldLabel
              ><Input
                id="user-password"
                v-model="password"
                :disabled="busy"
                type="password"
                required
                autocomplete="new-password"
                placeholder="至少 12 个字节"
            /></Field>
            <Field v-if="mode !== 'password'"
              ><FieldLabel for="user-role">角色</FieldLabel
              ><Select v-model="role" :disabled="busy"
                ><SelectTrigger id="user-role" class="w-full"><SelectValue /></SelectTrigger
                ><SelectContent
                  ><SelectItem value="member">普通用户</SelectItem
                  ><SelectItem value="admin">管理员</SelectItem></SelectContent
                ></Select
              ><FieldDescription
                >管理员可以管理所有用户和 Key。普通用户仅管理自己的 Key。</FieldDescription
              ></Field
            >
          </div></ScrollArea
        >
        <DialogFooter class="mt-6"
          ><Button type="button" variant="outline" :disabled="busy" @click="emit('close')"
            >取消</Button
          ><Button type="submit" :disabled="busy"
            ><LoaderCircle v-if="busy" class="size-4 animate-spin" />{{
              mode === 'create' ? '创建用户' : '保存修改'
            }}</Button
          ></DialogFooter
        >
      </form>
    </DialogContent>
  </Dialog>
</template>
