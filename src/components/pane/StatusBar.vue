<script setup lang="ts">
import { computed } from 'vue'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import HintList from './HintList.vue'

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
    <span v-if="p.search">{{ p.search.results.length.toLocaleString() }}{{ p.search.results.length >= 5000 ? '+' : '' }} found</span>
    <span v-else>{{ list.length.toLocaleString() }} items</span>
    <span v-if="p.selected.size" class="text-accent">{{ p.selected.size }} selected<template v-if="selectedBytes"> · {{ fmtSize(selectedBytes) }}</template></span>
    <!-- In split view the hints live once in the shared bar at the bottom (App.vue). -->
    <HintList v-if="!split" :pane="p" :split="false" class="ml-auto" />
  </footer>
</template>
