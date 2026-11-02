<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import type { IconName } from '../lib/icons'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, VisuallyHidden } from 'reka-ui'
import Icon from './Icon.vue'
import * as S from '../lib/store'
import { commands, prettyKeys } from '../lib/commands'
import { basename, fuzzy, keyLabel } from '../lib/util'
import { t } from '../lib/i18n'

type Item = { key: string; title: string; sub?: string; icon: IconName; keys?: string; group: string; run: () => unknown }

const state = S.state
const q = ref('')
const idx = ref(0)
const input = ref<HTMLInputElement>()
const open = computed({ get: () => !!state.palette, set: (v) => !v && (state.palette = null) })

watch(open, async (v) => {
  if (!v) return
  q.value = ''
  idx.value = 0
  await nextTick()
  input.value?.focus()
})

function folderItems(): Item[] {
  const paths = new Set<string>([...state.bookmarks, ...Object.keys(state.frecency), ...state.places.map((p) => p.path)])
  return [...paths].map((p) => ({
    key: 'f:' + p,
    title: basename(p) || p,
    sub: p,
    icon: state.bookmarks.includes(p) ? 'star' : 'folder',
    group: t('palette.folders'),
    run: () => S.go(S.pane(), p),
  }))
}

const items = computed<Item[]>(() => {
  const query = q.value.trim()
  const out: Item[] = []
  if (/^([~/]|[A-Za-z]:\\)/.test(query)) {
    const path = query.replace(/^~/, state.home)
    out.push({ key: 'go', title: t('palette.goTo', { path }), icon: 'enter', group: t('palette.path'), run: () => S.go(S.pane(), path) })
  }
  if (query.includes('*') && state.palette === 'all') {
    out.push({ key: 'sel', title: t('palette.selectMatching', { query }), icon: 'check-all', group: t('palette.select'), run: () => S.selectPattern(S.pane(), query) })
  }
  const folders = folderItems()
  if (state.palette === 'folders') {
    const scored = folders
      .map((f) => [query ? fuzzy(query, f.title) * 2 + Math.max(0, fuzzy(query, f.sub!)) : 0, f] as const)
      .filter(([s]) => s >= 0)
      .sort((a, b) => b[0] + S.frecencyScore(b[1].sub!) * 20 - (a[0] + S.frecencyScore(a[1].sub!) * 20))
    return [...out, ...scored.map(([, f]) => f)].slice(0, 60)
  }
  const cmds: Item[] = commands
    .filter((c) => c.id !== 'palette')
    .map((c) => ({ key: c.id, title: c.title, icon: c.icon, keys: c.keys, group: c.groupLabel, run: c.run }))
  if (!query) return [...out, ...cmds]
  const scoredCmds = cmds.map((c) => [fuzzy(query, c.title + ' ' + c.group), c] as const).filter(([s]) => s >= 0)
  const scoredFolders = folders.map((f) => [fuzzy(query, f.title) - 50 + S.frecencyScore(f.sub!), f] as const).filter(([s]) => s > -50)
  return [...out, ...[...scoredCmds, ...scoredFolders].sort((a, b) => b[0] - a[0]).map(([, x]) => x)].slice(0, 60)
})

watch(q, () => (idx.value = 0))

function run(item?: Item) {
  if (!item) return
  state.palette = null
  // Let the dialog close first so focus returns to the file list.
  setTimeout(() => item.run(), 0)
}

function key(e: KeyboardEvent) {
  if (e.key === 'ArrowDown') idx.value = Math.min(idx.value + 1, items.value.length - 1)
  else if (e.key === 'ArrowUp') idx.value = Math.max(idx.value - 1, 0)
  else if (e.key === 'Enter') run(items.value[idx.value])
  else return
  e.preventDefault()
  nextTick(() => document.querySelector('[data-palette-active]')?.scrollIntoView({ block: 'nearest' }))
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent
        class="fixed top-[12vh] left-1/2 z-50 w-[min(640px,92vw)] -translate-x-1/2 overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none"
        @open-auto-focus.prevent
      >
        <VisuallyHidden>
          <DialogTitle>{{ state.palette === 'folders' ? t('palette.jumpTitle') : t('palette.cmdTitle') }}</DialogTitle>
          <DialogDescription>{{ t('palette.desc') }}</DialogDescription>
        </VisuallyHidden>
        <div class="flex items-center gap-3 px-4 pt-1">
          <Icon :name="state.palette === 'folders' ? 'zap' : 'search'" class="size-4 text-accent" />
          <input
            ref="input" v-model="q"
            class="h-12 flex-1 bg-transparent text-[14px] outline-none placeholder:text-faint"
            :placeholder="state.palette === 'folders' ? t('palette.folderPh') : t('palette.cmdPh')"
            spellcheck="false" @keydown="key"
          />
        </div>
        <div class="max-h-[56vh] overflow-y-auto p-1.5">
          <template v-for="(it, i) in items" :key="it.key">
            <div v-if="i === 0 || items[i - 1].group !== it.group" class="px-2.5 pt-2 pb-1 text-[10.5px] font-medium uppercase tracking-wider text-faint">{{ it.group }}</div>
            <button
              class="flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-left text-[13px]"
              :class="i === idx ? 'bg-accent-soft text-fg' : 'text-dim'"
              :data-palette-active="i === idx || undefined"
              @mousemove="idx = i" @click="run(it)"
            >
              <Icon :name="it.icon" class="size-4 shrink-0" :class="i === idx && 'text-accent'" />
              <span class="truncate">{{ it.title }}</span>
              <span v-if="it.sub && it.sub !== it.title" class="truncate font-mono text-[11px] text-faint">{{ it.sub }}</span>
              <span v-if="it.keys" class="ml-auto flex shrink-0 gap-0.5"><kbd v-for="k in prettyKeys(it.keys)" :key="k">{{ k }}</kbd></span>
            </button>
          </template>
          <p v-if="!items.length" class="px-3 py-6 text-center text-faint">{{ t('palette.noMatches') }}</p>
        </div>
        <div class="flex gap-4 bg-raised/50 px-4 py-2 text-[11px] text-faint">
          <span><kbd>↑↓</kbd> {{ t('palette.choose') }}</span><span><kbd>Enter</kbd> {{ t('palette.run') }}</span><span><kbd>Esc</kbd> {{ t('palette.close') }}</span>
          <span class="ml-auto"><kbd>{{ keyLabel('Ctrl+P') }}</kbd> {{ t('palette.jump') }}</span>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
