<script setup lang="ts">
import { computed } from 'vue'
import {
  ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator,
  ContextMenuSub, ContextMenuSubTrigger, ContextMenuSubContent,
} from 'reka-ui'
import Icon from '../Icon.vue'
import TagsMenuItems from '../tags/TagsMenuItems.vue'
import * as S from '../../lib/store'
import type { Pane } from '../../lib/store'
import { keyLabel, parent } from '../../lib/util'
import { cmd, openProperties } from '../../lib/commands'
import { FORMATS, QUICK_FORMATS, caps, isImage, loadCaps, openConvert, quickConvert } from '../../lib/convert'

const props = defineProps<{ pane: Pane }>()
const onItem = defineModel<boolean>('onItem', { required: true })
const p = props.pane
const state = S.state
const cursorEntry = computed(() => S.visible(p)[p.cursor])
const paths = () => S.targets(p).map((e) => e.path)
// Right-click focuses this pane so keyboard shortcuts after the menu act on it too.
const focusThis = () => S.focusPane(S.tab().panes.indexOf(p))
const images = computed(() => {
  const t = S.targets(p)
  return t.length && t.every((e) => !e.isDir && isImage(e.path)) ? t.map((e) => e.path) : null
})
// Caps load lazily on first open; until then offer every quick format.
const quick = computed(() => QUICK_FORMATS.filter((f) => !caps.value || caps.value.formats.includes(f)))
const label = (ext: string) => FORMATS.find((f) => f.ext === ext)?.label ?? ext.toUpperCase()
function onOpen(o: boolean) {
  if (!o) return (onItem.value = false)
  focusThis()
  loadCaps().catch(() => {})
}

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
  <ContextMenuRoot @update:open="onOpen">
    <ContextMenuTrigger as-child @contextmenu.capture="focusThis"><slot /></ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent class="menu">
        <template v-if="onItem && cursorEntry">
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><Icon name="folder-open" />Open<span>Enter</span></ContextMenuItem>
          <ContextMenuItem v-if="p.search" class="mi" @select="S.open(p, cursorEntry, { reveal: true })"><Icon name="locate" />Show in folder<span>{{ keyLabel('Ctrl+Enter') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('tab')"><Icon name="plus" />Open in new tab</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('other')"><Icon name="columns" />Open in other pane</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.copyToClipboard('copy', p)"><Icon name="copy" />Copy<span>{{ keyLabel('Ctrl+C') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.copyToClipboard('move', p)"><Icon name="scissors" />Cut<span>{{ keyLabel('Ctrl+X') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.startRename(p)"><Icon name="pencil" />{{ S.targets(p).length > 1 ? `Rename ${S.targets(p).length} items…` : 'Rename' }}<span>F2</span></ContextMenuItem>
          <ContextMenuSub v-if="images">
            <ContextMenuSubTrigger class="mi"><Icon name="image" />Convert {{ images.length > 1 ? `${images.length} images ` : '' }}to<Icon name="chevron-right" class="ml-auto" /></ContextMenuSubTrigger>
            <ContextMenuPortal>
              <ContextMenuSubContent class="menu min-w-40" :side-offset="4">
                <ContextMenuItem v-for="f in quick" :key="f" class="mi" @select="quickConvert(images, f)">{{ label(f) }}</ContextMenuItem>
                <ContextMenuSeparator class="sep" />
                <ContextMenuItem class="mi" @select="openConvert(images)">More options…</ContextMenuItem>
              </ContextMenuSubContent>
            </ContextMenuPortal>
          </ContextMenuSub>
          <ContextMenuItem class="mi" @select="S.copyPath(p)"><Icon name="link" />Copy path<span>{{ keyLabel('Ctrl+Shift+C') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openProperties(p)"><Icon name="info" />Properties<span>{{ keyLabel('Alt+Enter') }}</span></ContextMenuItem>
          <template v-if="cursorEntry.isDir">
            <ContextMenuItem class="mi" @select="S.toggleBookmark(cursorEntry.path)"><Icon name="star" />{{ state.bookmarks.includes(cursorEntry.path) ? 'Remove from Favorites' : 'Add to Favorites' }}</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.computeSizes(S.targets(p).filter((x) => x.isDir).map((x) => x.path))"><Icon name="gauge" />Calculate size</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.openTerminal(cursorEntry.path)"><Icon name="terminal" />Open terminal here</ContextMenuItem>
          </template>
          <ContextMenuItem v-else class="mi" @select="S.reveal(cursorEntry.path)"><Icon name="eye" />Show in system file manager</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <TagsMenuItems :paths="paths()" />
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.trash(paths())"><Icon name="trash" />Move to Trash<span>{{ keyLabel(cmd('trash').keys!) }}</span></ContextMenuItem>
          <ContextMenuItem class="mi danger" @select="S.deleteForever(paths())"><Icon name="x" />Delete forever…<span>{{ keyLabel(cmd('delete').keys!) }}</span></ContextMenuItem>
        </template>
        <template v-else>
          <ContextMenuItem class="mi" @select="S.create('dir', p)"><Icon name="folder-plus" />New folder<span>{{ keyLabel('Ctrl+Shift+N') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.create('file', p)"><Icon name="file-plus" />New empty file<span>{{ keyLabel('Ctrl+Alt+N') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" :disabled="!state.clipboard" @select="S.paste(p)"><Icon name="paste" />Paste{{ state.clipboard ? ` ${state.clipboard.paths.length} item(s)` : '' }}<span>{{ keyLabel('Ctrl+V') }}</span></ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.selectAll(p)"><Icon name="check-all" />Select all<span>{{ keyLabel('Ctrl+A') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.showHidden = !state.showHidden"><Icon name="eye" />{{ state.showHidden ? 'Hide' : 'Show' }} hidden files<span>{{ keyLabel('Ctrl+H') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.calcFolderSizes(p)"><Icon name="gauge" />Show folder sizes<span>{{ keyLabel('Ctrl+Shift+S') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.toggleBookmark(p.path)"><Icon name="star" />{{ state.bookmarks.includes(p.path) ? 'Remove from Favorites' : 'Add to Favorites' }}<span>{{ keyLabel('Ctrl+D') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.openTerminal(p.path)"><Icon name="terminal" />Open terminal here<span>{{ keyLabel('Ctrl+`') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.properties = [p.path]"><Icon name="info" />Properties<span>{{ keyLabel('Alt+Enter') }}</span></ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
