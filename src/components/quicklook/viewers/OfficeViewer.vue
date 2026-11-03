<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../../Icon.vue'
import type { Entry } from '../../../lib/store'
import { t } from '../../../lib/i18n'
import OfficeBlocks, { type Block } from './OfficeBlocks.vue'

interface Sheet { name: string; rows: string[][]; truncated: boolean }
type Doc =
  | { kind: 'text'; blocks: Block[]; truncated: boolean }
  | { kind: 'slides'; slides: Block[][] }
  | { kind: 'sheets'; sheets: Sheet[] }

// Must match MAX_ROWS / MAX_COLS in src-tauri/src/docview.rs.
const MAX_ROWS = 1000
const MAX_COLS = 60

const props = defineProps<{ entry: Entry }>()
const doc = ref<Doc | null>(null)
const status = ref<'loading' | 'ok' | 'error'>('loading')
const sheetIx = ref(0)
let seq = 0

watch(
  () => props.entry.path,
  async (path) => {
    const my = ++seq
    status.value = 'loading'
    doc.value = null
    sheetIx.value = 0
    try {
      const d = await invoke<Doc>('office_preview', { path })
      if (my !== seq) return
      doc.value = d
      status.value = 'ok'
    } catch {
      if (my === seq) status.value = 'error'
    }
  },
  { immediate: true },
)

const empty = computed(() => {
  const d = doc.value
  if (!d) return false
  if (d.kind === 'text') return !d.blocks.length
  if (d.kind === 'slides') return !d.slides.some((s) => s.length)
  return !d.sheets.some((s) => s.rows.length)
})
const sheet = computed(() => (doc.value?.kind === 'sheets' ? doc.value.sheets[sheetIx.value] : undefined))
const colCount = computed(() => Math.max(0, ...(sheet.value?.rows.map((r) => r.length) ?? [])))
/** A, B, … Z, AA, AB … */
const colName = (i: number): string => (i < 26 ? '' : colName(Math.floor(i / 26) - 1)) + String.fromCharCode(65 + (i % 26))
</script>

<template>
  <div class="relative flex h-full w-full flex-col overflow-hidden bg-bg">
    <div v-if="status === 'loading'" class="grid flex-1 place-items-center"><Icon name="loader" class="size-6 animate-spin text-faint" /></div>
    <div v-else-if="status === 'error' || empty" class="grid flex-1 place-items-center text-center text-[13px] text-dim">
      <div>
        <Icon name="file-warning" class="mx-auto mb-2 size-10 text-faint" :stroke-width="1.25" />
        {{ status === 'error' ? t('ql.readError') : t('ql.emptyDoc') }}
      </div>
    </div>

    <template v-else-if="doc?.kind === 'sheets'">
      <div class="min-h-0 flex-1 overflow-auto select-text">
        <table v-if="sheet" class="border-separate border-spacing-0 text-[12px]">
          <thead>
            <tr>
              <th class="sticky top-0 left-0 z-20 h-6 min-w-10 border-r border-b border-line bg-panel" />
              <th v-for="c in colCount" :key="c" class="sticky top-0 z-10 h-6 border-r border-b border-line bg-panel px-2 font-normal text-faint">{{ colName(c - 1) }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in sheet.rows" :key="i">
              <td class="sticky left-0 z-10 border-r border-b border-line bg-panel px-2 text-right text-faint tabular-nums">{{ i + 1 }}</td>
              <td v-for="c in colCount" :key="c" class="max-w-80 truncate border-r border-b border-line px-2 py-1 align-top whitespace-pre" :title="r[c - 1]">{{ r[c - 1] }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="flex h-9 shrink-0 items-center gap-1 overflow-x-auto border-t border-line px-2 text-[12px]">
        <button
          v-for="(s, i) in doc.sheets" :key="i"
          class="h-6 shrink-0 rounded px-2.5 transition"
          :class="i === sheetIx ? 'bg-accent-soft text-fg' : 'text-dim hover:bg-raised hover:text-fg'"
          @click="sheetIx = i"
        >{{ s.name }}</button>
        <div class="flex-1" />
        <span v-if="sheet?.truncated" class="shrink-0 px-2 text-warn">{{ t('ql.sheetTruncated', { rows: MAX_ROWS, cols: MAX_COLS }) }}</span>
      </div>
    </template>

    <div v-else-if="doc" class="min-h-0 flex-1 overflow-auto select-text">
      <div class="mx-auto max-w-[780px] px-10 py-8">
        <p class="mb-6 text-[11.5px] text-faint">{{ t('ql.officeNote') }}</p>
        <OfficeBlocks v-if="doc.kind === 'text'" :blocks="doc.blocks" />
        <template v-else>
          <section v-for="(s, i) in doc.slides" :key="i" class="mb-5 rounded-lg bg-panel p-6 ring-1 ring-fg/5">
            <div class="mb-3 text-[11px] font-medium uppercase tracking-wide text-faint">{{ t('ql.slide', { n: i + 1 }) }}</div>
            <OfficeBlocks :blocks="s" />
          </section>
        </template>
        <p v-if="doc.kind === 'text' && doc.truncated" class="mt-6 text-[12px] text-warn">{{ t('ql.docTruncated') }}</p>
      </div>
    </div>
  </div>
</template>
