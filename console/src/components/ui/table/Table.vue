<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'
import { ScrollArea, ScrollBar } from '@/components/ui/scroll-area'

const props = defineProps<{
  class?: HTMLAttributes['class']
}>()
</script>

<template>
  <ScrollArea data-slot="table-container" class="relative w-full">
    <table data-slot="table" :class="cn('w-full caption-bottom text-sm', props.class)">
      <slot />
    </table>
    <ScrollBar orientation="horizontal" />
  </ScrollArea>
</template>
