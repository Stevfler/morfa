<script setup lang="ts">
import { computed, watch } from 'vue'
import Icon from '../Icon.vue'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { navKey } from '../../lib/keys'
import { basename, keyLabel } from '../../lib/util'
import { t } from '../../lib/i18n'

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
    searchTimer = setTimeout(() => S.startSearch(p, v), p.search?.source === 'index' ? 30 : 180) // the walk needs typing to pause; the index doesn't
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
    class="flex h-7 w-36 shrink-0 items-center gap-1.5 rounded-full px-3 transition focus-within:w-56 @2xl:w-60 @2xl:focus-within:w-80"
    :class="searching ? 'bg-accent-soft/40 ring-1 ring-accent/50' : 'bg-raised focus-within:ring-1 focus-within:ring-accent/50'"
  >
    <Icon name="search" v-if="searching" class="size-3.5 shrink-0 text-accent" />
    <Icon name="filter" v-else class="size-3.5 shrink-0 text-faint" />
    <input
      v-model="query" :data-filter="p.id"
      class="min-w-0 flex-1 bg-transparent text-[12.5px] outline-none placeholder:text-faint"
      :placeholder="searching ? t('pane.searchPlaceholder', { name: basename(p.path) || '/' }) : t('pane.filterPlaceholder')"
      spellcheck="false" @keydown="filterKey"
    />
    <Icon name="loader" v-if="p.search && !p.search.done" class="size-3.5 shrink-0 animate-spin text-accent" />
    <button v-if="query || searching" class="text-faint hover:text-fg" :aria-label="t('pane.clear')" @click="searching ? S.stopSearch(p) : (p.filter = '')"><Icon name="x" class="size-3.5" /></button>
    <button v-else class="hidden text-faint hover:text-accent @2xl:block" :title="t('pane.searchSubTitle', { key: keyLabel('Ctrl+F') })" @click="S.openSearch(p)"><kbd>{{ keyLabel('Ctrl+F') }}</kbd></button>
  </div>
</template>
