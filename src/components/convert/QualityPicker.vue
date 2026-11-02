<script setup lang="ts">
import { computed } from 'vue'
import { PRESETS } from '../../lib/convert'
import { t } from '../../lib/i18n'

const props = defineProps<{ lossless: boolean }>()
const quality = defineModel<number>('quality', { required: true })
const preset = defineModel<string>('preset', { required: true })

const opts = computed(() => [...PRESETS.map((p) => ({ key: p.key as string, label: p.label })), { key: 'custom', label: t('convert.custom') }])
function pick(key: string) {
  preset.value = key
  const p = PRESETS.find((x) => x.key === key)
  if (p) quality.value = p.quality
}
</script>

<template>
  <div v-if="props.lossless" class="flex items-center gap-2 rounded-lg bg-raised/60 px-3 py-2.5 text-[12px] text-dim">
    <span class="rounded bg-accent-soft px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide text-accent uppercase">{{ t('convert.lossless') }}</span>
    {{ t('convert.losslessDesc') }}
  </div>
  <div v-else class="space-y-3">
    <div class="inline-flex rounded-lg bg-raised/60 p-1" role="radiogroup" :aria-label="t('convert.quality')">
      <button
        v-for="o in opts" :key="o.key" type="button" role="radio" :aria-checked="preset === o.key"
        class="h-7 rounded-md px-3.5 text-[12.5px] transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
        :class="preset === o.key ? 'bg-panel text-fg shadow-sm' : 'text-dim hover:text-fg'"
        @click="pick(o.key)"
      >{{ o.label }}</button>
    </div>
    <div v-if="preset === 'custom'" class="flex items-center gap-3">
      <input v-model.number="quality" type="range" min="1" max="100" class="flex-1 accent-accent" :aria-label="t('convert.qualityValue')" />
      <span class="w-9 text-right text-[12.5px] text-fg tabular-nums">{{ quality }}</span>
    </div>
    <p v-else class="text-[11.5px] text-faint">{{ t('convert.qualityN', { n: quality }) }}</p>
  </div>
</template>
