<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  ArrowRight,
  Check,
  Eye,
  EyeOff,
  KeyRound,
  LoaderCircle,
  ShieldCheck,
  Users,
} from '@lucide/vue'
import { toast } from 'vue-sonner'
import Brand from '@/components/Brand.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { ScrollArea } from '@/components/ui/scroll-area'
import { api, messageOf } from '@/lib/api'
import { passwordError } from '@/lib/format'
import { session } from '@/lib/session'

const router = useRouter()
const route = useRoute()
const mode = ref('login')
const name = ref('')
const password = ref('')
const bootstrap = ref('')
const visible = ref(false)
const busy = ref(false)
const error = ref('')
const { notice } = session
const setup = computed(() => mode.value === 'setup')
watch(mode, () => {
  error.value = ''
  password.value = ''
  bootstrap.value = ''
  visible.value = false
})

async function submit() {
  if (busy.value) return
  error.value = ''
  if (!name.value.trim() || !password.value || (setup.value && !bootstrap.value.trim())) {
    error.value = '请填写所有必填信息。'
    return
  }
  if (setup.value) {
    error.value = passwordError(password.value)
    if (error.value) return
  }
  busy.value = true
  let created = false
  try {
    if (setup.value) {
      await api('/users', {
        method: 'POST',
        token: bootstrap.value.trim(),
        body: { login: name.value.trim(), password: password.value, administrator: true },
      })
      created = true
      bootstrap.value = ''
      toast.success('管理员账号已创建')
    }
    await session.login(name.value.trim(), password.value)
    password.value = ''
    const destination =
      typeof route.query.redirect === 'string' &&
      /^\/(?!\/)/.test(route.query.redirect) &&
      !route.query.redirect.startsWith('/login')
        ? route.query.redirect
        : '/'
    await router.replace(destination)
  } catch (e) {
    if (created) {
      mode.value = 'login'
      notice.value = '账号已创建，请使用刚设置的密码登录。'
    } else error.value = messageOf(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <ScrollArea class="h-svh">
    <div class="grid min-h-svh lg:grid-cols-[1fr_1.05fr]">
      <aside
        class="login-panel relative hidden flex-col justify-between overflow-hidden bg-[#132d29] p-12 text-white lg:flex xl:p-16"
      >
        <Brand inverse />
        <div class="relative z-10 mx-auto w-full max-w-md py-16">
          <span
            class="mb-7 inline-flex items-center gap-2 rounded-full border border-white/15 bg-white/5 px-3 py-1.5 text-xs text-emerald-100/90"
            ><span class="size-1.5 rounded-full bg-emerald-300" />YOUR TEAM. ONE GATEWAY.</span
          >
          <h1 class="text-[42px] leading-[1.35] font-medium tracking-tight xl:text-[48px]">
            连接 AI 的能力，<br /><span class="text-emerald-200/80">把访问管理好。</span>
          </h1>
          <p class="mt-6 max-w-sm text-sm leading-7 text-white/55">
            从一个用户、一把 Key 开始。让团队的每一次访问都有清晰的归属。
          </p>
          <div class="mt-12 space-y-5">
            <div
              v-for="item in [
                { icon: Users, title: '统一的用户管理', text: '管理团队成员与访问资格' },
                { icon: KeyRound, title: '独立的访问凭证', text: '为每个工具创建、管理和撤销 Key' },
                { icon: ShieldCheck, title: '清晰的权限边界', text: '个人使用与平台管理各司其职' },
              ]"
              :key="item.title"
              class="flex items-center gap-4"
            >
              <div
                class="flex size-10 items-center justify-center rounded-xl border border-white/10 bg-white/5"
              >
                <component :is="item.icon" class="size-[18px] text-emerald-200/80" />
              </div>
              <div>
                <p class="text-sm text-white/90">{{ item.title }}</p>
                <p class="mt-1 text-xs text-white/40">{{ item.text }}</p>
              </div>
            </div>
          </div>
        </div>
        <div class="relative z-10 flex items-center justify-between text-[11px] text-white/35">
          <span>AIGateway Console</span
          ><span class="flex items-center gap-1.5"><Check class="size-3" />简单 · 可控 · 专注</span>
        </div>
      </aside>
      <div class="flex min-h-svh flex-col justify-between bg-background px-6 py-8 sm:px-12">
        <div class="flex items-center justify-between">
          <Brand class="lg:invisible" /><span class="text-xs text-muted-foreground"
            >工作空间控制台</span
          >
        </div>
        <div class="mx-auto w-full max-w-[380px] py-12">
          <div class="mb-8">
            <p class="mb-3 text-[11px] font-medium tracking-[0.18em] text-primary">
              WELCOME TO AIGATEWAY
            </p>
            <h2 class="text-[30px] font-semibold tracking-tight">
              {{ setup ? '创建管理员账号' : '欢迎回来' }}
            </h2>
            <p class="mt-3 text-sm text-muted-foreground">
              {{
                setup ? '使用平台管理令牌，完成首次账号设置。' : '登录你的账号，继续管理工作空间。'
              }}
            </p>
          </div>
          <Tabs v-model="mode">
            <TabsList class="mb-6 w-full"
              ><TabsTrigger value="login" :disabled="busy" class="flex-1">账号登录</TabsTrigger
              ><TabsTrigger value="setup" :disabled="busy" class="flex-1"
                >首次设置</TabsTrigger
              ></TabsList
            >
            <TabsContent :value="mode">
              <form class="space-y-6" @submit.prevent="submit">
                <Alert v-if="error || notice" variant="destructive" role="alert"
                  ><AlertDescription>{{ error || notice }}</AlertDescription></Alert
                >
                <FieldGroup class="gap-5">
                  <Field v-if="setup"
                    ><FieldLabel for="bootstrap">平台管理令牌</FieldLabel
                    ><Input
                      id="bootstrap"
                      v-model="bootstrap"
                      type="password"
                      placeholder="输入管理员提供的令牌"
                      autocomplete="off"
                      :disabled="busy"
                      required
                    /><FieldDescription
                      >仅用于创建账号，不会保存在浏览器中。</FieldDescription
                    ></Field
                  >
                  <Field
                    ><FieldLabel for="login">用户名</FieldLabel
                    ><Input
                      id="login"
                      v-model="name"
                      placeholder="输入用户名"
                      autocomplete="username"
                      :disabled="busy"
                      maxlength="128"
                      required
                  /></Field>
                  <Field
                    ><FieldLabel for="password">密码</FieldLabel>
                    <div class="relative">
                      <Input
                        id="password"
                        v-model="password"
                        :type="visible ? 'text' : 'password'"
                        :placeholder="setup ? '设置至少 12 位密码' : '输入你的密码'"
                        :autocomplete="setup ? 'new-password' : 'current-password'"
                        :disabled="busy"
                        class="pr-11"
                        required
                      /><Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        class="absolute top-0.5 right-1"
                        :aria-label="visible ? '隐藏密码' : '显示密码'"
                        @click="visible = !visible"
                        ><EyeOff v-if="visible" class="size-4" /><Eye v-else class="size-4"
                      /></Button></div
                  ></Field>
                </FieldGroup>
                <Button type="submit" class="h-10 w-full" :disabled="busy"
                  ><LoaderCircle v-if="busy" class="size-4 animate-spin" />{{
                    busy ? '正在处理…' : setup ? '创建并登录' : '登录控制台'
                  }}<ArrowRight v-if="!busy" class="ml-auto size-4"
                /></Button>
              </form>
            </TabsContent>
          </Tabs>
          <Card class="mt-7 border-dashed bg-muted/30 shadow-none"
            ><CardContent class="flex gap-3 p-4! text-xs leading-5 text-muted-foreground"
              ><ShieldCheck class="mt-0.5 size-4 shrink-0" />
              <p>
                {{
                  setup
                    ? '管理令牌来自网关部署配置。已有账号时，请直接使用账号登录。'
                    : '账号由管理员创建。如需开通账号或重置密码，请联系工作空间管理员。'
                }}
              </p></CardContent
            ></Card
          >
        </div>
        <p class="text-center text-[11px] text-muted-foreground/60">
          一个入口，清晰管理每一次访问。
        </p>
      </div>
    </div>
  </ScrollArea>
</template>
