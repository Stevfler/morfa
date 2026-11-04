<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import Icon from '../Icon.vue'
import type { IconName } from '../../lib/icons'
import TagDots from '../tags/TagDots.vue'
import * as S from '../../lib/store'
import type { Entry, Pane } from '../../lib/store'
import { fmtDate, fmtDateFull, fmtSize, kindOf, KIND_COLOR, KIND_ICON, KIND_LABEL, parent } from '../../lib/util'
import { t } from '../../lib/i18n'
import { branchOf } from '../../lib/git'

const props = defineProps<{ pane: Pane; e: Entry; i: number; row: number; active: boolean; dropTarget: string | null }>()
const p = props.pane
const state = S.state

const renameInput = ref<HTMLInputElement>()
watch(renameInput, (el) => {
  if (!el) return
  el.focus()
  const name = el.value
  const dot = name.lastIndexOf('.')
  // Select only the stem so the extension survives — what you almost always want.
  el.setSelectionRange(0, dot > 0 && !props.e.isDir ? dot : name.length)
})
function renameKey(ev: KeyboardEvent) {
  if (ev.key === 'Enter') S.renameTo(props.e.path, (ev.target as HTMLInputElement).value, props.pane)
  else if (ev.key === 'Escape') state.renaming = null
  ev.stopPropagation()
}

const CLOUD_ICON: Record<string, IconName> = { synced: 'cloud-check', pending: 'refresh', partial: 'cloud-download' }
const CLOUD_CLASS: Record<string, string> = { synced: 'text-ok', pending: 'text-accent', partial: 'text-dim' }
const cloudTitle = (m: string) =>
  t(m === 'synced' ? 'cloud.markSynced' : m === 'pending' ? 'cloud.markPending' : 'cloud.markPartial')

const branch = computed(() => (props.e.isDir ? branchOf(props.e.path) : null))

const rel = (e: Entry) => {
  const dir = parent(e.path) ?? ''
  return dir.startsWith(p.path) ? dir.slice(p.path.length).replace(/^[\\/]/, '') || '.' : dir
}
const cut = (path: string) => state.clipboard?.mode === 'move' && state.clipboard.paths.includes(path)
function sizeCell(e: Entry) {
  if (!e.isDir) return fmtSize(e.size)
  const s = state.sizes[e.path]
  return s === 'pending' ? '…' : typeof s === 'number' ? fmtSize(s) : ''
}
</script>

<template>
  <div
    class="file-cols absolute inset-x-1 grid cursor-default items-center gap-2 rounded-md px-2 text-[12.5px]"
    :style="{ top: `${i * row}px`, height: `${row - 2}px` }"
    :class="[
      p.selected.has(e.path) ? 'bg-accent/25' : i === p.cursor && active ? 'bg-raised' : 'hover:bg-raised/50',
      i === p.cursor && active && 'ring-1 ring-inset ring-accent/70',
      dropTarget === e.path && 'bg-accent-soft ring-1 ring-inset ring-accent',
      (e.hidden || cut(e.path)) && 'opacity-50',
    ]"
    draggable="true"
  >
    <div class="flex min-w-0 items-center gap-2.5">
      <Icon :name="KIND_ICON[kindOf(e)]" class="size-4 shrink-0" :style="{ color: KIND_COLOR[kindOf(e)] }" :tint="e.isDir ? 0.18 : 0" />
      <input
        v-if="state.renaming === e.path" ref="renameInput" :value="e.name"
        class="h-6 min-w-0 flex-1 rounded border border-accent bg-panel px-1.5 outline-none"
        spellcheck="false" @keydown="renameKey" @blur="S.renameTo(e.path, ($event.target as HTMLInputElement).value, pane)" @mousedown.stop
      />
      <span v-else class="truncate">{{ e.name }}</span>
      <span v-if="branch" class="flex max-w-36 min-w-6 shrink-[4] items-center gap-1 rounded bg-raised px-1.5 py-px text-[11px] text-dim" :title="t('preview.branch') + ': ' + branch">
        <Icon name="git-branch" class="size-3 shrink-0" />
        <span class="truncate">{{ branch }}</span>
      </span>
      <TagDots :path="e.path" />
      <Icon v-if="e.cloud" :name="CLOUD_ICON[e.cloud] ?? 'cloud'" class="size-3.5 shrink-0" :class="CLOUD_CLASS[e.cloud]" :title="cloudTitle(e.cloud)" />
      <Icon v-else-if="e.link" name="external-link" class="size-3.5 shrink-0 text-faint" :title="t('cloud.onlineOnly')" />
      <Icon v-else-if="e.cloud === '' && S.isSpecial(e.path)" name="cloud" class="size-3.5 shrink-0 text-faint/60" :title="t('cloud.markRemote')" />
      <Icon name="link" v-if="e.isSymlink" class="size-3 shrink-0 text-faint" />
      <span v-if="p.search" class="truncate text-[11.5px] text-faint">{{ rel(e) }}</span>
    </div>
    <span class="text-right tabular-nums text-dim">{{ sizeCell(e) }}</span>
    <span class="truncate text-right text-dim" :title="fmtDateFull(e.modified)">{{ fmtDate(e.modified) }}</span>
    <span class="truncate text-right text-faint">{{ e.isDir ? t('pane.kindFolder') : e.ext ? e.ext.toUpperCase() : KIND_LABEL[kindOf(e)] }}</span>
  </div>
</template>
