<script setup lang="ts">
import { computed } from 'vue'
import {
  ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator,
} from 'reka-ui'
import {
  X, FolderPlus, Gauge, FolderOpen, Copy, Scissors, ClipboardPaste, Pencil, Trash2, Link, Star, SquareTerminal,
  FilePlus, CheckCheck, Eye, Columns2, Plus, Locate,
} from 'lucide-vue-next'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { parent } from '../../lib/util'

const props = defineProps<{ pane: Pane }>()
const onItem = defineModel<boolean>('onItem', { required: true })
const p = props.pane
const state = S.state
const cursorEntry = computed(() => S.visible(p)[p.cursor])
const paths = () => S.targets(p).map((e) => e.path)
// Right-click focuses this pane so keyboard shortcuts after the menu act on it too.
const focusThis = () => S.focusPane(S.tab().panes.indexOf(p))

function openIn(where: 'tab' | 'other') {
  const e = cursorEntry.value
  if (!e) return
  const dir = e.isDir ? e.path : parent(e.path)!
  if (where === 'tab') S.newTab(dir)
  else {
    if (!S.otherPane()) S.toggleSplit()
    S.go(S.otherPane()!, dir)
  }
}
</script>

<template>
  <ContextMenuRoot @update:open="(o: boolean) => (o ? focusThis() : (onItem = false))">
    <ContextMenuTrigger as-child @contextmenu.capture="focusThis"><slot /></ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent class="menu">
        <template v-if="onItem && cursorEntry">
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><FolderOpen />Open<span>Enter</span></ContextMenuItem>
          <ContextMenuItem v-if="p.search" class="mi" @select="S.open(p, cursorEntry, { reveal: true })"><Locate />Show in folder<span>Ctrl+Enter</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('tab')"><Plus />Open in new tab</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('other')"><Columns2 />Open in other pane</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.copyToClipboard('copy', p)"><Copy />Copy<span>Ctrl+C</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.copyToClipboard('move', p)"><Scissors />Cut<span>Ctrl+X</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.startRename(p)"><Pencil />{{ S.targets(p).length > 1 ? `Rename ${S.targets(p).length} items…` : 'Rename' }}<span>F2</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.copyPath(p)"><Link />Copy path<span>Ctrl+Shift+C</span></ContextMenuItem>
          <template v-if="cursorEntry.isDir">
            <ContextMenuItem class="mi" @select="S.toggleBookmark(cursorEntry.path)"><Star />{{ state.bookmarks.includes(cursorEntry.path) ? 'Remove from Favorites' : 'Add to Favorites' }}</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.computeSizes(S.targets(p).filter((x) => x.isDir).map((x) => x.path))"><Gauge />Calculate size</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.openTerminal(cursorEntry.path)"><SquareTerminal />Open terminal here</ContextMenuItem>
          </template>
          <ContextMenuItem v-else class="mi" @select="S.reveal(cursorEntry.path)"><Eye />Show in system file manager</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.trash(paths())"><Trash2 />Move to Trash<span>Del</span></ContextMenuItem>
          <ContextMenuItem class="mi danger" @select="S.deleteForever(paths())"><X />Delete forever…<span>Shift+Del</span></ContextMenuItem>
        </template>
        <template v-else>
          <ContextMenuItem class="mi" @select="S.create('dir', p)"><FolderPlus />New folder<span>Ctrl+Shift+N</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.create('file', p)"><FilePlus />New empty file<span>Ctrl+Alt+N</span></ContextMenuItem>
          <ContextMenuItem class="mi" :disabled="!state.clipboard" @select="S.paste(p)"><ClipboardPaste />Paste{{ state.clipboard ? ` ${state.clipboard.paths.length} item(s)` : '' }}<span>Ctrl+V</span></ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.selectAll(p)"><CheckCheck />Select all<span>Ctrl+A</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.showHidden = !state.showHidden"><Eye />{{ state.showHidden ? 'Hide' : 'Show' }} hidden files<span>Ctrl+H</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.calcFolderSizes(p)"><Gauge />Show folder sizes<span>Ctrl+Shift+S</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.toggleBookmark(p.path)"><Star />{{ state.bookmarks.includes(p.path) ? 'Remove from Favorites' : 'Add to Favorites' }}<span>Ctrl+D</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.openTerminal(p.path)"><SquareTerminal />Open terminal here<span>Ctrl+`</span></ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
