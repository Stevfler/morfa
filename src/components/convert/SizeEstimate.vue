<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import Icon from '../Icon.vue'
import { estimate, type ConvertOpts } from '../../lib/convert'
import { fmtSize } from '../../lib/util'
import { t } from '../../lib/i18n'

const props = defineProps<{ source: string | null; opts: ConvertOpts; disabled?: boolean }>()
const result = ref<{ before: number; after: number } | null>(null)
const busy = ref(false)
let seq = 0
let timer = 0

// Output folder and name don't change the size, so editing them doesn't re-estimate.
const key = () => JSON.stringify({ ...props.opts, outDir: null, nameTemplate: '' })
watch(() => [props.source, key(), props.disabled] as const, () => {
  clearTimeout(timer)
  const id = ++seq
  if (!props.source || props.disabled) return void ((result.value = null), (busy.value = false))
  busy.value = true
  timer = window.setTimeout(async () => {
    try {
      const r = await estimate(props.source!, props.opts)
      if (id === seq) result.value = r
    } catch {
      if (id === seq) result.value = null
    } finally {
      if (id === seq) busy.value = false
    }
  }, 300)
}, { immediate: true })
onBeforeUnmount(() => (seq++, clearTimeout(timer)))

const pct = (r: { before: number; after: number }) => {
  const d = r.before ? Math.round(((r.after - r.before) / r.before) * 100) : 0
  return d <= 0 ? `−${-d}%` : `+${d}%`
}
</script>

<template>
  <div v-if="result || busy" class="flex items-center gap-2 text-[12px] text-dim tabular-nums" aria-live="polite">
    <Icon v-if="busy" name="loader" class="size-3.5 animate-spin text-faint" />
    <template v-if="result">
      <span>{{ fmtSize(result.before) }}</span>
      <Icon name="arrow-right" class="size-3 text-faint" />
      <span class="text-fg">≈ {{ fmtSize(result.after) }}</span>
      <span :class="result.after <= result.before ? 'text-accent' : 'text-warn'">({{ pct(result) }})</span>
    </template>
    <span v-else class="text-faint">{{ t('convert.estimating') }}</span>
  </div>
</template>
