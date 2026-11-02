<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ConvertOpts } from '../../lib/convert'

const model = defineModel<ConvertOpts['resize']>({ required: true })
const CHOICES = [
  { key: 'orig', label: 'Original', v: null },
  { key: 'p50', label: '50%', v: { mode: 'percent', value: 50 } },
  { key: 'm3840', label: '3840 px', v: { mode: 'max', value: 3840 } },
  { key: 'm1920', label: '1920 px', v: { mode: 'max', value: 1920 } },
  { key: 'm1280', label: '1280 px', v: { mode: 'max', value: 1280 } },
] as const
const custom = ref(false)
const px = ref(model.value?.mode === 'max' ? model.value.value : 2560)

const active = computed(() => {
  if (custom.value) return 'custom'
  const m = model.value
  return CHOICES.find((c) => (c.v === null ? m === null : m?.mode === c.v.mode && m.value === c.v.value))?.key ?? 'custom'
})
function pick(c: (typeof CHOICES)[number]) {
  custom.value = false
  model.value = c.v ? { ...c.v } : null
}
function setCustom() {
  custom.value = true
  model.value = { mode: 'max', value: Math.max(16, px.value || 0) }
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-2">
    <button
      v-for="c in CHOICES" :key="c.key" type="button"
      class="h-8 rounded-full px-3.5 text-[12.5px] transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
      :class="active === c.key ? 'bg-accent-soft text-accent' : 'bg-raised/60 text-dim hover:bg-raised hover:text-fg'"
      @click="pick(c)"
    >{{ c.label }}</button>
    <button
      type="button" class="h-8 rounded-full px-3.5 text-[12.5px] transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
      :class="active === 'custom' ? 'bg-accent-soft text-accent' : 'bg-raised/60 text-dim hover:bg-raised hover:text-fg'"
      @click="setCustom"
    >Custom</button>
    <label v-if="active === 'custom'" class="flex items-center gap-1.5 text-[12px] text-dim">
      max
      <input
        v-model.number="px" type="number" min="16" max="65535" aria-label="Maximum side in pixels"
        class="h-8 w-20 rounded-md border border-line bg-bg px-2 text-[12.5px] text-fg outline-none focus:border-accent/60"
        @input="setCustom"
      />
      px
    </label>
  </div>
</template>
