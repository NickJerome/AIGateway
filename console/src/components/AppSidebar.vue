<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  Activity,
  ScrollText,
  ChartColumn,
  Users,
  Blocks,
  Puzzle,
  KeyRound,
  UserRound,
  ShieldCheck,
  ChevronsUpDown,
  LogOut,
  ArrowUpRight,
} from '@lucide/vue'
import { toast } from 'vue-sonner'
import { plugins, pluginPath } from '@/lib/plugins'
import Brand from '@/components/Brand.vue'
import { session } from '@/lib/session'
import {
  Sidebar,
  SidebarHeader,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  useSidebar,
} from '@/components/ui/sidebar'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
} from '@/components/ui/dropdown-menu'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'

const { user, isAdmin } = session
const { items: installed, loading: pluginsLoading, error: pluginsError } = plugins
onMounted(() => {
  if (isAdmin.value) void plugins.refresh()
})
const route = useRoute()
const router = useRouter()
const { setOpenMobile } = useSidebar()
const groups = computed(() => [
  ...(isAdmin.value
    ? [
        {
          label: '工作空间',
          links: [
            { path: '/plugins', title: '插件管理', icon: Blocks },
            { path: '/users', title: '用户管理', icon: Users },
            { path: '/keys', title: 'Key 管理', icon: KeyRound },
            { path: '/traces', title: '调用记录', icon: Activity },
            { path: '/logs', title: '日志', icon: ScrollText },
            { path: '/usage', title: '用量', icon: ChartColumn },
          ],
        },
      ]
    : []),
  {
    label: '个人',
    links: [
      { path: '/my-keys', title: '我的 Key', icon: KeyRound },
      { path: '/account', title: '我的账号', icon: UserRound },
    ],
  },
])
async function logout() {
  try {
    await session.logout()
  } catch {
    toast.info('已退出当前设备；远端会话将在到期后失效。')
  }
  await router.replace('/login')
}
</script>

<template>
  <Sidebar collapsible="offcanvas" class="border-sidebar-border">
    <SidebarHeader class="gap-6 px-6 pt-7 pb-5">
      <RouterLink to="/" aria-label="AIGateway 首页" @click="setOpenMobile(false)"
        ><Brand
      /></RouterLink>
      <div
        class="flex items-center gap-3 rounded-lg border border-sidebar-border bg-background/80 p-3"
      >
        <div class="flex size-8 items-center justify-center rounded-md bg-muted">
          <ShieldCheck class="size-4 text-muted-foreground" />
        </div>
        <div class="min-w-0 flex-1">
          <p class="text-sm font-medium">默认工作空间</p>
          <p class="mt-0.5 text-[11px] text-muted-foreground">访问与身份管理</p>
        </div>
      </div>
    </SidebarHeader>
    <SidebarContent class="overflow-hidden">
      <ScrollArea class="h-full">
        <SidebarGroup v-for="group in groups" :key="group.label" class="px-4 pb-5">
          <SidebarGroupLabel
            class="mb-2 px-3 text-[11px] tracking-wider text-muted-foreground/80"
            >{{ group.label }}</SidebarGroupLabel
          >
          <SidebarMenu class="gap-1">
            <SidebarMenuItem v-for="link in group.links" :key="link.path">
              <SidebarMenuButton
                as-child
                :is-active="route.path === link.path"
                class="h-10 px-3 data-active:font-medium"
              >
                <RouterLink :to="link.path" @click="setOpenMobile(false)"
                  ><component :is="link.icon" /><span>{{ link.title }}</span
                  ><span
                    v-if="route.path === link.path"
                    class="ml-auto size-1.5 rounded-full bg-primary"
                /></RouterLink>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>
        <SidebarGroup v-if="isAdmin" class="px-4 pb-5">
          <SidebarGroupLabel class="mb-2 px-3 text-[11px]">插件快捷入口</SidebarGroupLabel>
          <p v-if="pluginsError" class="px-3 text-xs text-destructive">
            插件列表加载失败，请在插件管理中重试。
          </p>
          <p v-else-if="!installed.length" class="px-3 text-xs text-muted-foreground">
            {{ pluginsLoading ? '加载中…' : '安装插件后，入口会出现在这里。' }}
          </p>
          <SidebarMenu class="gap-1">
            <SidebarMenuItem v-for="plugin in installed" :key="plugin.name">
              <SidebarMenuButton
                as-child
                :is-active="route.path === pluginPath(plugin.name)"
                class="h-10 px-3"
              >
                <RouterLink :to="pluginPath(plugin.name)" @click="setOpenMobile(false)"
                  ><Puzzle /><span class="truncate">{{ plugin.name }}</span
                  ><span
                    :class="[
                      'ml-auto size-1.5 shrink-0 rounded-full',
                      plugin.error
                        ? 'bg-destructive'
                        : plugin.enabled
                          ? 'bg-primary'
                          : 'bg-muted-foreground/40',
                    ]"
                    :aria-label="plugin.enabled ? '运行中' : '已停用'"
                /></RouterLink>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>
      </ScrollArea>
    </SidebarContent>
    <SidebarFooter class="gap-4 p-4">
      <div class="rounded-xl border border-sidebar-border bg-background/70 p-4">
        <div class="mb-2 flex items-center justify-between">
          <span class="text-xs font-medium">让访问井然有序</span
          ><ShieldCheck class="size-4 text-primary" />
        </div>
        <p class="text-xs leading-5 text-muted-foreground">
          为每个工具创建独立 Key，随时管理与撤销访问。
        </p>
        <RouterLink
          to="/my-keys"
          class="mt-3 inline-flex items-center gap-1 text-xs font-medium text-primary"
          @click="setOpenMobile(false)"
          >管理我的 Key <ArrowUpRight class="size-3"
        /></RouterLink>
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <SidebarMenuButton size="lg" class="px-2">
            <Avatar class="size-9 rounded-lg"
              ><AvatarFallback class="rounded-lg bg-primary/10 text-primary">{{
                user?.login.slice(0, 2).toUpperCase()
              }}</AvatarFallback></Avatar
            >
            <div class="min-w-0 flex-1 text-left">
              <p class="truncate text-sm font-medium">{{ user?.login }}</p>
              <p class="mt-0.5 text-xs text-muted-foreground">
                {{ isAdmin ? '系统管理员' : '普通用户' }}
              </p>
            </div>
            <ChevronsUpDown class="size-4 text-muted-foreground" />
          </SidebarMenuButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent side="top" align="end" class="w-56">
          <DropdownMenuLabel class="flex items-center justify-between"
            >{{ user?.login
            }}<Badge variant="secondary" class="text-[10px]">{{
              isAdmin ? '管理员' : '用户'
            }}</Badge></DropdownMenuLabel
          >
          <DropdownMenuSeparator />
          <DropdownMenuItem @select="router.push('/account')"
            ><UserRound />我的账号</DropdownMenuItem
          >
          <DropdownMenuItem @select="logout"><LogOut />退出登录</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </SidebarFooter>
  </Sidebar>
</template>
