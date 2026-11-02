<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../../Icon.vue'
import type { Entry } from '../../../lib/store'
import { fmtSize } from '../../../lib/util'
import { t } from '../../../lib/i18n'

const MAX = 512 * 1024
const props = defineProps<{ entry: Entry }>()
const text = ref<string | null>(null)
const status = ref<'loading' | 'ok' | 'binary' | 'error'>('loading')
const wrap = ref(false)
let seq = 0

watch(
  () => props.entry.path,
  async (path) => {
    const my = ++seq
    status.value = 'loading'
    text.value = null
    try {
      const s = await invoke<string | null>('read_text', { path, max: MAX })
      if (my !== seq) return
      text.value = s
      status.value = s == null ? 'binary' : 'ok'
    } catch {
      if (my === seq) status.value = 'error'
    }
  },
  { immediate: true },
)

const truncated = computed(() => props.entry.size > MAX)
const lines = computed(() => (text.value ?? '').split('\n'))
</script>

<template>
  <div class="relative flex h-full w-full flex-col overflow-hidden bg-bg">
    <div v-if="status === 'loading'" class="grid flex-1 place-items-center"><Icon name="loader" class="size-6 animate-spin text-faint" /></div>
    <div v-else-if="status !== 'ok'" class="grid flex-1 place-items-center text-center text-[13px] text-dim">
      <div>
        <Icon name="file-warning" class="mx-auto mb-2 size-10 text-faint" :stroke-width="1.25" />
        {{ status === 'binary' ? t('ql.binary') : t('ql.readError') }}
      </div>
    </div>
    <template v-else>
      <div class="flex h-8 shrink-0 items-center gap-2 px-3 text-[11.5px] text-faint">
        <span v-if="truncated" class="text-warn">{{ t('ql.truncated', { max: fmtSize(MAX), size: fmtSize(entry.size) }) }}</span>
        <span v-else>{{ t('ql.lines', { count: lines.length }) }}</span>
        <div class="flex-1" />
        <button class="tb" :class="wrap && 'on'" @click="wrap = !wrap"><Icon name="wrap-text" class="size-3.5" />{{ t('ql.wrap') }}</button>
      </div>
      <div class="min-h-0 flex-1 overflow-auto font-mono text-[12px] leading-[1.6] select-text">
        <table class="border-collapse" :class="wrap ? 'w-full table-fixed' : 'min-w-full'">
          <tr v-for="(l, i) in lines" :key="i">
            <td class="sticky left-0 w-[1%] bg-bg pr-3 pl-4 text-right align-top whitespace-nowrap text-faint select-none">{{ i + 1 }}</td>
            <td class="pr-4 align-top text-fg" :class="wrap ? 'break-all whitespace-pre-wrap' : 'whitespace-pre'">{{ l || ' ' }}</td>
          </tr>
        </table>
      </div>
    </template>
  </div>
</template>

<style scoped>
@reference '../../../style.css';
.tb {
  @apply inline-flex h-6 items-center gap-1 rounded px-1.5 text-dim transition hover:bg-raised hover:text-fg;
}
.tb.on {
  @apply text-accent;
}
</style>
