<script setup lang="ts">
import { computed } from 'vue'
import { FORMATS, formatGroup, isMedia, type FormatGroup } from '../../lib/convert'
import { t } from '../../lib/i18n'

/** `formats`: valid targets for the selection (backend order); null while loading. */
const props = defineProps<{ formats: string[] | null; paths: string[] }>()
const model = defineModel<string>({ required: true })
const list = computed(() =>
  (props.formats ?? []).map((ext) => FORMATS.find((f) => f.ext === ext) ?? { ext, label: ext.toUpperCase(), hint: '' }),
)
// Media sources: split into Video / Audio rows (gif sits with video); otherwise one unlabeled grid.
const groups = computed(() => {
  if (!props.paths.some(isMedia)) return [{ key: null as FormatGroup | null, items: list.value }]
  const by = new Map<FormatGroup, typeof list.value>()
  for (const f of list.value) {
    const g = f.ext === 'gif' ? 'video' : formatGroup(f.ext)
    by.set(g, [...(by.get(g) ?? []), f])
  }
  return [...by].map(([key, items]) => ({ key: by.size > 1 ? key : null, items }))
})
</script>

<template>
  <p v-if="!formats" class="text-[12px] text-faint">{{ t('convert.loadingTargets') }}</p>
  <p v-else-if="!list.length" class="text-[12px] text-faint">{{ t('convert.noTargets') }}</p>
  <template v-else>
  <div v-for="g in groups" :key="g.key ?? '-'" class="not-first:mt-3">
  <p v-if="g.key" class="mb-1.5 text-[11px] text-faint">{{ t(`convert.group.${g.key}`) }}</p>
  <div class="grid grid-cols-[repeat(auto-fill,minmax(118px,1fr))] gap-2" role="radiogroup" :aria-label="t('convert.outputFormat')">
    <button
      v-for="f in g.items" :key="f.ext" type="button" role="radio" :aria-checked="model === f.ext"
      class="flex flex-col items-start gap-0.5 rounded-lg px-3 py-2.5 text-left transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60 disabled:opacity-40"
      :class="model === f.ext ? 'bg-accent-soft ring-1 ring-accent/50' : 'bg-raised/60 hover:bg-raised'"
      @click="model = f.ext"
    >
      <span class="text-[13px] font-medium" :class="model === f.ext ? 'text-accent' : 'text-fg'">{{ f.label }}</span>
      <span class="line-clamp-2 text-[11px] leading-snug text-faint">{{ f.hint }}</span>
    </button>
  </div>
  </div>
  </template>
</template>
