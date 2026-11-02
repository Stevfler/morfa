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
import { t as tr } from '../../lib/i18n'
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
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><Icon name="folder-open" />{{ tr('pane.menu.open') }}<span>Enter</span></ContextMenuItem>
          <ContextMenuItem v-if="p.search" class="mi" @select="S.open(p, cursorEntry, { reveal: true })"><Icon name="locate" />{{ tr('pane.menu.showInFolder') }}<span>{{ keyLabel('Ctrl+Enter') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('tab')"><Icon name="plus" />{{ tr('pane.menu.openNewTab') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('other')"><Icon name="columns" />{{ tr('pane.menu.openOtherPane') }}</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.copyToClipboard('copy', p)"><Icon name="copy" />{{ tr('pane.menu.copy') }}<span>{{ keyLabel('Ctrl+C') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.copyToClipboard('move', p)"><Icon name="scissors" />{{ tr('pane.menu.cut') }}<span>{{ keyLabel('Ctrl+X') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.startRename(p)"><Icon name="pencil" />{{ S.targets(p).length > 1 ? tr('pane.menu.renameN', { count: S.targets(p).length }) : tr('pane.menu.rename') }}<span>F2</span></ContextMenuItem>
          <ContextMenuSub v-if="images">
            <ContextMenuSubTrigger class="mi"><Icon name="image" />{{ images.length > 1 ? tr('pane.menu.convertNTo', { count: images.length }) : tr('pane.menu.convertTo') }}<Icon name="chevron-right" class="ml-auto" /></ContextMenuSubTrigger>
            <ContextMenuPortal>
              <ContextMenuSubContent class="menu min-w-40" :side-offset="4">
                <ContextMenuItem v-for="f in quick" :key="f" class="mi" @select="quickConvert(images, f)">{{ label(f) }}</ContextMenuItem>
                <ContextMenuSeparator class="sep" />
                <ContextMenuItem class="mi" @select="openConvert(images)">{{ tr('pane.menu.moreOptions') }}</ContextMenuItem>
              </ContextMenuSubContent>
            </ContextMenuPortal>
          </ContextMenuSub>
          <ContextMenuItem class="mi" @select="S.copyPath(p)"><Icon name="link" />{{ tr('pane.menu.copyPath') }}<span>{{ keyLabel('Ctrl+Shift+C') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openProperties(p)"><Icon name="info" />{{ tr('pane.menu.properties') }}<span>{{ keyLabel('Alt+Enter') }}</span></ContextMenuItem>
          <template v-if="cursorEntry.isDir">
            <ContextMenuItem class="mi" @select="S.toggleBookmark(cursorEntry.path)"><Icon name="star" />{{ state.bookmarks.includes(cursorEntry.path) ? tr('pane.menu.removeFav') : tr('pane.menu.addFav') }}</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.computeSizes(S.targets(p).filter((x) => x.isDir).map((x) => x.path))"><Icon name="gauge" />{{ tr('pane.menu.calcSize') }}</ContextMenuItem>
            <ContextMenuItem class="mi" @select="S.openTerminal(cursorEntry.path)"><Icon name="terminal" />{{ tr('pane.menu.terminalHere') }}</ContextMenuItem>
          </template>
          <ContextMenuItem v-else class="mi" @select="S.reveal(cursorEntry.path)"><Icon name="eye" />{{ tr('pane.menu.showInSystem') }}</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <TagsMenuItems :paths="paths()" />
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.trash(paths())"><Icon name="trash" />{{ tr('pane.menu.trash') }}<span>{{ keyLabel(cmd('trash').keys!) }}</span></ContextMenuItem>
          <ContextMenuItem class="mi danger" @select="S.deleteForever(paths())"><Icon name="x" />{{ tr('pane.menu.deleteForever') }}<span>{{ keyLabel(cmd('delete').keys!) }}</span></ContextMenuItem>
        </template>
        <template v-else>
          <ContextMenuItem class="mi" @select="S.create('dir', p)"><Icon name="folder-plus" />{{ tr('pane.menu.newFolder') }}<span>{{ keyLabel('Ctrl+Shift+N') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.create('file', p)"><Icon name="file-plus" />{{ tr('pane.menu.newFile') }}<span>{{ keyLabel('Ctrl+Alt+N') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" :disabled="!state.clipboard" @select="S.paste(p)"><Icon name="paste" />{{ state.clipboard ? tr('pane.menu.pasteN', { count: state.clipboard.paths.length }) : tr('pane.menu.paste') }}<span>{{ keyLabel('Ctrl+V') }}</span></ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.selectAll(p)"><Icon name="check-all" />{{ tr('pane.menu.selectAll') }}<span>{{ keyLabel('Ctrl+A') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.showHidden = !state.showHidden"><Icon name="eye" />{{ state.showHidden ? tr('pane.menu.hideHidden') : tr('pane.menu.showHidden') }}<span>{{ keyLabel('Ctrl+H') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.calcFolderSizes(p)"><Icon name="gauge" />{{ tr('pane.menu.folderSizes') }}<span>{{ keyLabel('Ctrl+Shift+S') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.toggleBookmark(p.path)"><Icon name="star" />{{ state.bookmarks.includes(p.path) ? tr('pane.menu.removeFav') : tr('pane.menu.addFav') }}<span>{{ keyLabel('Ctrl+D') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.openTerminal(p.path)"><Icon name="terminal" />{{ tr('pane.menu.terminalHere') }}<span>{{ keyLabel('Ctrl+`') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.properties = [p.path]"><Icon name="info" />{{ tr('pane.menu.properties') }}<span>{{ keyLabel('Alt+Enter') }}</span></ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
