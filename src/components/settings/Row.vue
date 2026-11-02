<script setup lang="ts">
defineProps<{ title: string; desc?: string }>()
</script>

<template>
  <div class="flex items-center justify-between gap-4 py-2.5">
    <div class="min-w-0">
      <div class="text-[12.5px] text-fg">{{ title }}</div>
      <div v-if="desc" class="mt-0.5 text-[11.5px] text-dim">{{ desc }}</div>
    </div>
    <div class="shrink-0"><slot /></div>
  </div>
</template>
