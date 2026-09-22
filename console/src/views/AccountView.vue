<script setup lang="ts">
import { KeyRound, ArrowUpRight, ShieldCheck, Copy } from '@lucide/vue'
import { toast } from 'vue-sonner'
import PageHeading from '@/components/PageHeading.vue'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { session } from '@/lib/session'
import { dateTime } from '@/lib/format'
const { user, isAdmin } = session
async function copyId() {
  try {
    await navigator.clipboard.writeText(user.value!.id)
    toast.success('账号 ID 已复制')
  } catch {
    toast.error('复制失败，请手动选择并复制账号 ID。')
  }
}
</script>
<template>
  <div class="space-y-7">
    <PageHeading title="我的账号" description="查看你的账号信息与访问权限。" eyebrow="PERSONAL" />
    <div class="grid items-start gap-6 xl:grid-cols-[1.4fr_1fr]">
      <Card class="shadow-none"
        ><CardHeader
          ><CardTitle>账号信息</CardTitle
          ><CardDescription>此账号由工作空间管理员管理。</CardDescription></CardHeader
        ><CardContent v-if="user" class="space-y-6">
          <div class="flex items-center gap-4">
            <Avatar class="size-14 rounded-2xl"
              ><AvatarFallback class="rounded-2xl bg-primary/10 text-lg text-primary">{{
                user.login.slice(0, 2).toUpperCase()
              }}</AvatarFallback></Avatar
            >
            <div>
              <p class="text-lg font-semibold">{{ user.login }}</p>
              <Badge variant="outline" class="status-success mt-2">已启用</Badge>
            </div>
          </div>
          <Separator />
          <dl class="space-y-5 text-sm">
            <div>
              <dt class="mb-2 text-muted-foreground">账号 ID</dt>
              <dd class="flex items-center gap-2">
                <span class="min-w-0 break-all font-mono text-xs">{{ user.id }}</span
                ><Button variant="ghost" size="icon-sm" aria-label="复制账号 ID" @click="copyId"
                  ><Copy class="size-3.5"
                /></Button>
              </dd>
            </div>
            <div class="flex justify-between gap-4">
              <dt class="text-muted-foreground">账号角色</dt>
              <dd>{{ isAdmin ? '管理员' : '普通用户' }}</dd>
            </div>
            <div class="flex flex-wrap justify-between gap-4">
              <dt class="text-muted-foreground">创建时间</dt>
              <dd>{{ dateTime(user.created_at) }}</dd>
            </div>
          </dl>
        </CardContent></Card
      >
      <div class="space-y-5">
        <Card class="shadow-none"
          ><CardHeader
            ><KeyRound class="mb-2 size-5 text-primary" /><CardTitle>管理你的访问凭证</CardTitle
            ><CardDescription
              >为不同的应用创建独立 Key，便于识别和撤销。</CardDescription
            ></CardHeader
          ><CardContent
            ><Button variant="outline" as-child
              ><RouterLink to="/my-keys"
                >前往我的 Key<ArrowUpRight /></RouterLink></Button></CardContent></Card
        ><Card class="border-dashed bg-muted/30 shadow-none"
          ><CardContent class="flex gap-3 p-5!"
            ><ShieldCheck class="mt-0.5 size-5 shrink-0 text-muted-foreground" />
            <div>
              <p class="text-sm font-medium">需要修改密码？</p>
              <p class="mt-2 text-xs leading-6 text-muted-foreground">
                {{
                  isAdmin
                    ? '你可以在用户管理中重置自己的密码。修改后需要重新登录。'
                    : '请联系管理员重置密码。修改后需要重新登录，现有 Key 不受影响。'
                }}
              </p>
              <Button v-if="isAdmin" variant="link" class="mt-2 h-auto p-0" as-child
                ><RouterLink to="/users">前往用户管理<ArrowUpRight /></RouterLink
              ></Button></div></CardContent
        ></Card>
      </div>
    </div>
  </div>
</template>
