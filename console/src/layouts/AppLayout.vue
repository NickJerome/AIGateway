<script setup lang="ts">
import { useRoute } from 'vue-router'
import { useColorMode } from '@vueuse/core'
import { ChevronRight, Moon, Sun, ShieldCheck } from '@lucide/vue'
import AppSidebar from '@/components/AppSidebar.vue'
import { SidebarProvider, SidebarInset, SidebarTrigger } from '@/components/ui/sidebar'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

const route = useRoute()
const mode = useColorMode({ initialValue: 'light' })
</script>

<template>
  <SidebarProvider class="h-svh min-h-0 overflow-hidden" :style="{ '--sidebar-width': '15.5rem' }">
    <AppSidebar />
    <SidebarInset class="h-svh min-w-0 overflow-hidden bg-background">
      <header
        class="flex h-[68px] shrink-0 items-center justify-between gap-4 border-b px-4 sm:px-8"
      >
        <div class="flex min-w-0 items-center gap-3">
          <SidebarTrigger class="text-muted-foreground" aria-label="切换导航" />
          <Separator orientation="vertical" class="mr-1 h-4!" />
          <span class="hidden text-sm text-muted-foreground sm:block">控制台</span
          ><ChevronRight class="hidden size-3.5 text-muted-foreground/60 sm:block" />
          <span class="truncate text-sm font-medium">{{ route.meta.title }}</span>
        </div>
        <div class="flex items-center gap-4">
          <span class="hidden items-center gap-1.5 text-xs text-muted-foreground sm:inline-flex"
            ><ShieldCheck class="size-3.5 text-primary" />访问管理</span
          >
          <Tooltip
            ><TooltipTrigger as-child
              ><Button
                variant="ghost"
                size="icon"
                :aria-label="mode === 'dark' ? '切换浅色模式' : '切换深色模式'"
                @click="mode = mode === 'dark' ? 'light' : 'dark'"
                ><Sun v-if="mode === 'dark'" class="size-4" /><Moon
                  v-else
                  class="size-4" /></Button></TooltipTrigger
            ><TooltipContent>切换外观</TooltipContent></Tooltip
          >
        </div>
      </header>
      <ScrollArea :key="route.path" class="min-h-0 flex-1" data-testid="page-scroll-area">
        <main id="main-content" class="mx-auto w-full max-w-[1440px] space-y-7 p-5 sm:p-8 lg:p-10">
          <RouterView :key="route.path" />
          <footer
            class="flex items-center justify-between gap-3 pt-3 pb-1 text-[11px] text-muted-foreground/70"
          >
            <span>AIGateway Console</span><span>更简单的团队 AI 访问管理</span>
          </footer>
        </main>
      </ScrollArea>
    </SidebarInset>
  </SidebarProvider>
</template>
