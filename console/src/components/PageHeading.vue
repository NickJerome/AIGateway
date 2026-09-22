<script setup lang="ts">
defineProps<{ title: string; description: string; eyebrow?: string }>()
</script>

<template>
  <div class="flex flex-wrap items-end justify-between gap-4">
    <div class="space-y-2">
      <p
        v-if="eyebrow"
        class="text-[11px] font-medium tracking-[0.18em] text-muted-foreground uppercase"
      >
        {{ eyebrow }}
      </p>
      <h1 class="text-2xl font-semibold tracking-tight sm:text-[28px]">{{ title }}</h1>
      <p class="text-sm leading-relaxed text-muted-foreground">{{ description }}</p>
    </div>
    <div class="flex items-center gap-2"><slot /></div>
  </div>
</template>
