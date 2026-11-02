<script setup lang="ts">
import { computed } from 'vue'
import { FORMATS, caps } from '../../lib/convert'

const model = defineModel<string>({ required: true })
const list = computed(() => {
  const c = caps.value
  return c ? FORMATS.filter((f) => c.formats.includes(f.ext)) : FORMATS
})
</script>

<template>
  <div class="grid grid-cols-[repeat(auto-fill,minmax(118px,1fr))] gap-2" role="radiogroup" aria-label="Output format">
    <button
      v-for="f in list" :key="f.ext" type="button" role="radio" :aria-checked="model === f.ext" :disabled="!caps"
      class="flex flex-col items-start gap-0.5 rounded-lg px-3 py-2.5 text-left transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60 disabled:opacity-40"
      :class="model === f.ext ? 'bg-accent-soft ring-1 ring-accent/50' : 'bg-raised/60 hover:bg-raised'"
      @click="model = f.ext"
    >
      <span class="text-[13px] font-medium" :class="model === f.ext ? 'text-accent' : 'text-fg'">{{ f.label }}</span>
      <span class="line-clamp-2 text-[11px] leading-snug text-faint">{{ f.hint }}</span>
    </button>
  </div>
</template>
