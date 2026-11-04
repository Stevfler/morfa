<script setup lang="ts">
import { computed } from 'vue'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import HintList from './HintList.vue'
import { t } from '../../lib/i18n'

const props = defineProps<{ pane: Pane; split: boolean }>()
const p = props.pane
const state = S.state

const list = computed(() => S.visible(p))
const selectedBytes = computed(() =>
  list.value.filter((e) => p.selected.has(e.path)).reduce((s, e) => s + (e.isDir ? ((state.sizes[e.path] as number) || 0) : e.size), 0),
)
</script>

<template>
  <footer class="flex h-8 shrink-0 items-center gap-3 px-4 text-[11.5px] text-faint">
    <template v-if="p.search">
      <span v-if="p.search.total != null && p.search.total > p.search.results.length && p.search.done">{{ t('pane.foundOf', { n: p.search.results.length.toLocaleString(), total: p.search.total.toLocaleString() }) }}</span>
      <span v-else>{{ t('pane.found', { count: p.search.results.length, n: p.search.results.length.toLocaleString() + (p.search.results.length >= 5000 ? '+' : '') }) }}</span>
      <span v-if="p.search.source && !(p.search.source === 'live' && p.search.reason === 'off')" class="rounded bg-raised px-1.5 py-px text-[10.5px]" :class="p.search.source === 'index' ? 'text-accent' : ''" :title="p.search.source === 'index' ? t('pane.source.indexHint') : t(`pane.source.why.${p.search.reason ?? 'off'}` as 'pane.source.why.off')">{{ t(`pane.source.${p.search.source}`) }}</span>
    </template>
    <span v-else>{{ t('pane.items', { count: list.length, n: list.length.toLocaleString() }) }}</span>
    <span v-if="p.selected.size" class="text-accent">{{ t('pane.selected', { count: p.selected.size, n: p.selected.size }) }}<template v-if="selectedBytes"> · {{ fmtSize(selectedBytes) }}</template></span>
    <!-- In split view the hints live once in the shared bar at the bottom (App.vue). -->
    <HintList v-if="!split" :pane="p" :split="false" class="ml-auto" />
  </footer>
</template>
