<script setup lang="ts">
import { computed, watch } from 'vue'
import { Search, ListFilter, X, LoaderCircle } from 'lucide-vue-next'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { navKey } from '../../lib/keys'
import { basename, keyLabel } from '../../lib/util'

const props = defineProps<{ pane: Pane }>()
const p = props.pane

const searching = computed(() => !!p.search)
let searchTimer: ReturnType<typeof setTimeout> | undefined
const query = computed({
  get: () => (p.search ? p.search.query : p.filter),
  set: (v: string) => {
    if (p.search) {
      p.search.query = v // the watcher below debounces the actual search
    } else {
      p.filter = v
      p.cursor = 0
    }
  },
})
watch(() => p.search?.query, (v, old) => {
  // Typed-from-anywhere characters also trigger the search.
  if (v != null && v !== old) {
    clearTimeout(searchTimer)
    searchTimer = setTimeout(() => S.startSearch(p, v), 180)
  }
})
function filterKey(ev: KeyboardEvent) {
  if (ev.key === 'Escape') {
    if (p.search) S.stopSearch(p)
    else p.filter = ''
    ;(ev.target as HTMLElement).blur()
    ev.stopPropagation()
    return
  }
  if (navKey(p, ev)) {
    if (ev.key === 'Enter') (ev.target as HTMLElement).blur()
    ev.stopPropagation()
  }
}
</script>

<template>
  <!-- filter / deep search -->
  <div
    class="flex h-7 w-60 items-center gap-1.5 rounded-md border px-2 transition focus-within:w-80"
    :class="searching ? 'border-accent/60 bg-accent-soft/40' : 'border-line bg-panel focus-within:border-accent/60'"
  >
    <Search v-if="searching" class="size-3.5 shrink-0 text-accent" />
    <ListFilter v-else class="size-3.5 shrink-0 text-faint" />
    <input
      v-model="query" :data-filter="p.id"
      class="min-w-0 flex-1 bg-transparent text-[12.5px] outline-none placeholder:text-faint"
      :placeholder="searching ? `Search inside ${basename(p.path) || '/'}… (*.pdf works)` : 'Filter — just start typing'"
      spellcheck="false" @keydown="filterKey"
    />
    <LoaderCircle v-if="p.search && !p.search.done" class="size-3.5 shrink-0 animate-spin text-accent" />
    <button v-if="query || searching" class="text-faint hover:text-fg" @click="searching ? S.stopSearch(p) : (p.filter = '')"><X class="size-3.5" /></button>
    <button v-else class="text-faint hover:text-accent" :title="`Search subfolders too (${keyLabel('Ctrl+F')})`" @click="S.openSearch(p)"><kbd>{{ keyLabel('Ctrl+F') }}</kbd></button>
  </div>
</template>
