<script setup lang="ts">
import { computed } from 'vue'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import { cmd, prettyKeys } from '../../lib/commands'

const props = defineProps<{ pane: Pane; split: boolean }>()
const p = props.pane
const state = S.state

const list = computed(() => S.visible(p))
const selectedBytes = computed(() =>
  list.value.filter((e) => p.selected.has(e.path)).reduce((s, e) => s + (e.isDir ? ((state.sizes[e.path] as number) || 0) : e.size), 0),
)
const hint = computed(() => {
  if (state.clipboard) return [['Ctrl+V', `paste ${state.clipboard.paths.length} here`], ['Esc', 'forget clipboard']]
  if (p.search) return [['Enter', 'open'], ['Ctrl+Enter', 'show in folder'], ['Esc', 'exit search']]
  if (p.selected.size > 1) return [['F2', 'batch rename'], ['Ctrl+C', 'copy'], [cmd('trash').keys!, 'trash'], ...(props.split ? [['F7', 'copy → other pane']] : [])]
  if (p.filter) return [['↑↓', 'pick'], ['Enter', 'open'], ['Esc', 'clear filter']]
  return [['type', 'to filter'], ['Ctrl+F', 'deep search'], ['Space', 'preview'], ['Ctrl+K', 'all actions']]
})
</script>

<template>
  <footer class="flex h-7 shrink-0 items-center gap-3 border-t border-line px-3 text-[11.5px] text-faint">
    <span v-if="p.search">{{ p.search.results.length.toLocaleString() }}{{ p.search.results.length >= 5000 ? '+' : '' }} found</span>
    <span v-else>{{ list.length.toLocaleString() }} items</span>
    <span v-if="p.selected.size" class="text-accent">{{ p.selected.size }} selected<template v-if="selectedBytes"> · {{ fmtSize(selectedBytes) }}</template></span>
    <span class="ml-auto flex items-center gap-3 overflow-hidden whitespace-nowrap">
      <span v-for="[k, t] in hint" :key="k" class="flex items-center gap-1"><kbd v-for="kk in prettyKeys(k)" :key="kk">{{ kk }}</kbd>{{ t }}</span>
    </span>
  </footer>
</template>
