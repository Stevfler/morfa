<script setup lang="ts">
import { computed, ref } from 'vue'
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
import { canDownload, refreshOffice } from '../../lib/office'
import { pin, unpin, showLocal, showInCloud, openWeb, syncNow } from '../../lib/cloud'
import { within } from '../../lib/store/navigation'
import { formatLabel, isConvertible, isImage, openConvert, quickConvert, targetsFor } from '../../lib/convert'

const props = defineProps<{ pane: Pane }>()
const onItem = defineModel<boolean>('onItem', { required: true })
const p = props.pane
const state = S.state
const cursorEntry = computed(() => S.visible(p)[p.cursor])
const paths = () => S.targets(p).map((e) => e.path)
// Right-click focuses this pane so keyboard shortcuts after the menu act on it too.
const focusThis = () => S.focusPane(S.tab().panes.indexOf(p))
const convertible = computed(() => {
  const t = S.targets(p)
  return t.length && t.every((e) => !e.isDir && isConvertible(e.path)) ? t.map((e) => e.path) : null
})
const convertLabel = computed(() => {
  const c = convertible.value!
  if (c.length === 1) return tr('pane.menu.convertTo')
  return c.every(isImage) ? tr('pane.menu.convertNTo', { count: c.length }) : tr('pane.menu.convertNFilesTo', { count: c.length })
})
// Targets load lazily on menu open (cached per extension set); null while loading.
const quick = ref<string[] | null>(null)
let seq = 0
async function loadQuick() {
  const c = convertible.value
  const my = ++seq
  quick.value = null
  if (!c) return
  const list = await targetsFor(c).catch(() => [])
  if (my === seq) quick.value = list.slice(0, 4)
}
// Documents with no formats because the Office module is missing: offer it instead of an empty list.
const getOffice = computed(() => {
  const c = convertible.value
  return !!c && quick.value?.length === 0 && !c.every(isImage) && canDownload()
})
function onOpen(o: boolean) {
  if (!o) return (onItem.value = false)
  focusThis()
  void refreshOffice()
  void loadQuick()
}

const cloud = computed(() => S.parseCloud(p.path))
const inArchive = computed(() => S.parseArchive(p.path))
const archives = computed(() => S.targets(p).filter((e) => !e.isDir && S.isArchiveFile(e.name)).map((e) => e.path))
const otherFolder = computed(() => {
  const o = S.otherPane()
  return o && o !== p && !S.isSpecial(o.path) ? o.path : null
})
const mirrored = computed(() => !cloud.value && state.cloud.some((a) => within(p.path, a.localRoot)))
const canPin = computed(() => S.targets(p).some((e) => !e.link && (!e.cloud || e.cloud === 'partial')))
const canUnpin = computed(() => S.targets(p).some((e) => !!e.cloud))

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
        <template v-if="inArchive && onItem && cursorEntry">
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><Icon name="folder-open" />{{ tr('pane.menu.open') }}<span>Enter</span></ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.extractEntries(p, S.targets(p))"><Icon name="file-archive" />{{ tr('archive.extractHere') }}</ContextMenuItem>
          <ContextMenuItem v-if="otherFolder" class="mi" @select="S.extractEntries(p, S.targets(p), otherFolder!)"><Icon name="columns" />{{ tr('archive.extractOther') }}</ContextMenuItem>
        </template>
        <template v-else-if="inArchive">
          <ContextMenuItem class="mi" @select="S.extractArchives([inArchive.archive])"><Icon name="file-archive" />{{ tr('archive.extractAll') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.selectAll(p)"><Icon name="check-all" />{{ tr('pane.menu.selectAll') }}<span>{{ keyLabel('Ctrl+A') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.showHidden = !state.showHidden"><Icon name="eye" />{{ state.showHidden ? tr('pane.menu.hideHidden') : tr('pane.menu.showHidden') }}<span>{{ keyLabel('Ctrl+H') }}</span></ContextMenuItem>
        </template>
        <template v-else-if="cloud && onItem && cursorEntry">
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><Icon name="folder-open" />{{ tr('pane.menu.open') }}<span>Enter</span></ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem v-if="canPin" class="mi" @select="pin(S.targets(p))"><Icon name="cloud-download" />{{ tr('cloud.pin') }}</ContextMenuItem>
          <template v-if="canUnpin">
            <ContextMenuItem class="mi" @select="unpin(S.targets(p), false)"><Icon name="cloud" />{{ tr('cloud.unpin') }}</ContextMenuItem>
            <ContextMenuItem class="mi" @select="unpin(S.targets(p), true)"><Icon name="broom" />{{ tr('cloud.unpinDelete') }}</ContextMenuItem>
          </template>
          <ContextMenuItem v-if="cursorEntry.cloud" class="mi" @select="showLocal(cursorEntry)"><Icon name="hard-drive" />{{ tr('cloud.showLocal') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openWeb(cloud.id, cursorEntry.isDir ? S.parseCloud(cursorEntry.path)!.rel : cloud.rel)"><Icon name="external-link" />{{ tr('cloud.openWeb') }}</ContextMenuItem>
        </template>
        <template v-else-if="cloud">
          <ContextMenuItem class="mi" @select="syncNow(cloud.id)"><Icon name="refresh" />{{ tr('cloud.syncNow') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openWeb(cloud.id, cloud.rel)"><Icon name="external-link" />{{ tr('cloud.openWeb') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.selectAll(p)"><Icon name="check-all" />{{ tr('pane.menu.selectAll') }}<span>{{ keyLabel('Ctrl+A') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="state.showHidden = !state.showHidden"><Icon name="eye" />{{ state.showHidden ? tr('pane.menu.hideHidden') : tr('pane.menu.showHidden') }}<span>{{ keyLabel('Ctrl+H') }}</span></ContextMenuItem>
        </template>
        <template v-else-if="onItem && cursorEntry">
          <ContextMenuItem class="mi" @select="S.open(p, cursorEntry)"><Icon name="folder-open" />{{ tr('pane.menu.open') }}<span>Enter</span></ContextMenuItem>
          <ContextMenuItem v-if="p.search" class="mi" @select="S.open(p, cursorEntry, { reveal: true })"><Icon name="locate" />{{ tr('pane.menu.showInFolder') }}<span>{{ keyLabel('Ctrl+Enter') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('tab')"><Icon name="plus" />{{ tr('pane.menu.openNewTab') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openIn('other')"><Icon name="columns" />{{ tr('pane.menu.openOtherPane') }}</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" @select="S.copyToClipboard('copy', p)"><Icon name="copy" />{{ tr('pane.menu.copy') }}<span>{{ keyLabel('Ctrl+C') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.copyToClipboard('move', p)"><Icon name="scissors" />{{ tr('pane.menu.cut') }}<span>{{ keyLabel('Ctrl+X') }}</span></ContextMenuItem>
          <ContextMenuItem class="mi" @select="S.startRename(p)"><Icon name="pencil" />{{ S.targets(p).length > 1 ? tr('pane.menu.renameN', { count: S.targets(p).length }) : tr('pane.menu.rename') }}<span>F2</span></ContextMenuItem>
          <ContextMenuSub v-if="convertible">
            <ContextMenuSubTrigger class="mi"><Icon name="image" />{{ convertLabel }}<Icon name="chevron-right" class="ml-auto" /></ContextMenuSubTrigger>
            <ContextMenuPortal>
              <ContextMenuSubContent class="menu min-w-40" :side-offset="4">
                <ContextMenuItem v-if="!quick" class="mi" disabled>…</ContextMenuItem>
                <ContextMenuItem v-if="getOffice" class="mi" @select="openConvert(convertible)"><Icon name="download" />{{ tr('office.getModule') }}</ContextMenuItem>
                <ContextMenuItem v-for="f in quick ?? []" :key="f" class="mi" @select="quickConvert(convertible, f)">{{ formatLabel(f) }}</ContextMenuItem>
                <ContextMenuSeparator v-if="quick?.length !== 0 || getOffice" class="sep" />
                <ContextMenuItem class="mi" @select="openConvert(convertible)">{{ tr('pane.menu.moreOptions') }}</ContextMenuItem>
              </ContextMenuSubContent>
            </ContextMenuPortal>
          </ContextMenuSub>
          <template v-if="archives.length">
            <ContextMenuItem class="mi" @select="S.extractArchives(archives)"><Icon name="file-archive" />{{ tr('archive.extractHere') }}</ContextMenuItem>
            <ContextMenuItem v-if="otherFolder" class="mi" @select="S.extractArchives(archives, otherFolder!)"><Icon name="columns" />{{ tr('archive.extractOther') }}</ContextMenuItem>
          </template>
          <ContextMenuSub v-if="!p.search && !S.isSpecial(p.path)">
            <ContextMenuSubTrigger class="mi"><Icon name="file-archive" />{{ tr('archive.compress') }}<Icon name="chevron-right" class="ml-auto" /></ContextMenuSubTrigger>
            <ContextMenuPortal>
              <ContextMenuSubContent class="menu min-w-40" :side-offset="4">
                <ContextMenuItem class="mi" @select="S.compress(paths(), p.path, 'zip')">ZIP</ContextMenuItem>
                <ContextMenuItem class="mi" @select="S.compress(paths(), p.path, 'tar.gz')">TAR.GZ</ContextMenuItem>
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
          <template v-if="mirrored">
            <ContextMenuSeparator class="sep" />
            <ContextMenuItem class="mi" @select="showInCloud(cursorEntry.path)"><Icon name="cloud" />{{ tr('cloud.showInCloud') }}</ContextMenuItem>
            <ContextMenuItem v-if="canPin" class="mi" @select="pin(S.targets(p))"><Icon name="cloud-check" />{{ tr('cloud.pin') }}</ContextMenuItem>
            <ContextMenuItem v-if="canUnpin" class="mi" @select="unpin(S.targets(p), false)"><Icon name="cloud" />{{ tr('cloud.unpin') }}</ContextMenuItem>
          </template>
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
