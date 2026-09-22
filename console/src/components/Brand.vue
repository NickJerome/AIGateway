<script setup lang="ts">
import { Layers3 } from '@lucide/vue'
defineProps<{ inverse?: boolean; compact?: boolean }>()
</script>

<template>
  <div class="flex items-center gap-2.5" :class="inverse ? 'text-white' : 'text-foreground'">
    <span
      class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm"
    >
      <Layers3 class="size-5" :stroke-width="1.8" />
    </span>
    <span v-if="!compact" class="text-lg font-semibold tracking-tight"
      >AIGateway<span class="ml-0.5 text-primary">.</span></span
    >
  </div>
</template>
