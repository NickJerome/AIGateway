<script setup lang="ts">
import { ChevronLeft, ChevronRight } from '@lucide/vue'
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationPrevious,
  PaginationNext,
  PaginationEllipsis,
} from '@/components/ui/pagination'
defineProps<{ total: number; pageSize: number }>()
const page = defineModel<number>({ required: true })
</script>

<template>
  <div class="flex flex-wrap items-center justify-between gap-3 border-t px-5 py-4">
    <p class="text-xs text-muted-foreground">
      共 {{ total }} 条记录<span v-if="total"
        >，第 {{ Math.min((page - 1) * pageSize + 1, total) }}–{{
          Math.min(page * pageSize, total)
        }}
        条</span
      >
    </p>
    <Pagination
      class="mx-0 w-auto"
      v-model:page="page"
      :items-per-page="pageSize"
      :total="total"
      :sibling-count="1"
      show-edges
    >
      <PaginationContent v-slot="{ items }">
        <PaginationPrevious aria-label="上一页"
          ><ChevronLeft /><span class="hidden sm:block">上一页</span></PaginationPrevious
        >
        <template v-for="(item, index) in items" :key="index">
          <PaginationItem
            v-if="item.type === 'page'"
            :value="item.value"
            :is-active="item.value === page"
            >{{ item.value }}</PaginationItem
          >
          <PaginationEllipsis v-else :index="index" />
        </template>
        <PaginationNext aria-label="下一页"
          ><span class="hidden sm:block">下一页</span><ChevronRight
        /></PaginationNext>
      </PaginationContent>
    </Pagination>
  </div>
</template>
