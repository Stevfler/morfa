<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent } from 'reka-ui'
import * as S from '../../lib/store'
import type { Entry } from '../../lib/store'
import { canQuickLook, fmtSize, kindOf, OFFICE, type Kind } from '../../lib/util'
import { scrollCursorIntoView, spaceHeldOpen } from '../../lib/keys'
import { settings } from '../../lib/settings'
import QuickLookHeader from './QuickLookHeader.vue'
import Icon from '../Icon.vue'
import { t } from '../../lib/i18n'
import { imageUrl } from '../../composables/useImageUrl'

import Image from './viewers/ImageViewer.vue'
const Text = defineAsyncComponent(() => import('./viewers/TextViewer.vue'))
const Office = defineAsyncComponent(() => import('./viewers/OfficeViewer.vue'))
const VIEWERS: Partial<Record<Kind, unknown>> = { image: Image, text: Text, code: Text }

const open = computed({
  get: () => S.state.quickLook && !!entry.value,
  set: (v) => close(v),
})
const motion = computed(() => `ql-motion-${settings.motion}`)

function close(v = false) {
  S.state.quickLook = v
}
const entry = computed<Entry | undefined>(() => {
  const p = S.pane()
  return S.visible(p)[p.cursor]
})
const viewer = computed(() => (!entry.value ? null : OFFICE.includes(entry.value.ext) ? Office : VIEWERS[kindOf(entry.value)]))
const pos = computed(() => `${S.pane().cursor + 1} / ${S.visible(S.pane()).length}`)

const viewable = computed(() => !!viewer.value && canQuickLook(entry.value))
const tooBig = computed(() => !!entry.value && S.archiveTooBig(entry.value))

// Viewers read from disk, so an archive entry goes through a temp copy; the header keeps the original entry.
const unpacked = shallowRef<{ from: string; path: string | null } | null>(null)
let unpackTimer: ReturnType<typeof setTimeout> | undefined
let unpackSeq = 0
watch(
  () => (open.value && viewable.value && !tooBig.value && S.parseArchive(entry.value!.path) ? entry.value!.path : null),
  (path, old) => {
    clearTimeout(unpackTimer)
    const my = ++unpackSeq
    // Moving or closing only: on opening, the panel may be unpacking this very entry, so join it.
    if (old) S.cancelPreviewUnpack()
    if (!path) return // the copy stays on screen through the closing animation
    unpacked.value = null
    // Debounced so arrowing through an archive doesn't start and cancel an unpack per row.
    unpackTimer = setTimeout(async () => {
      const local = await S.previewLocal(path).catch(() => null)
      if (my === unpackSeq) unpacked.value = { from: path, path: local }
    }, 70)
  },
  { immediate: true },
)
const inArchive = computed(() => !!entry.value && !!S.parseArchive(entry.value.path))
const done = computed(() => (unpacked.value?.from === entry.value?.path ? unpacked.value : null))
/** What the viewer reads; null while unpacking or when there's nothing to show. */
const shown = computed<Entry | null>(() => {
  const e = entry.value
  if (!e || !viewable.value || tooBig.value) return null
  if (!inArchive.value) return e
  return done.value?.path ? { ...e, path: done.value.path } : null
})
const pending = computed(() => viewable.value && !tooBig.value && inArchive.value && !done.value)
const noPreview = computed(() => (!viewable.value ? t('ql.noPreview') : tooBig.value ? t('ql.archiveTooBig', { max: fmtSize(S.ARCHIVE_PREVIEW_MAX) }) : t('ql.readError')))

// Folder emptied or navigated away under us: close rather than show nothing.
watch(entry, (e) => !e && close())

// Warm the image cache for the cursor (so Space is instant) and, while open, its neighbours.
let warm: ReturnType<typeof setTimeout> | undefined
watch(entry, () => {
  clearTimeout(warm)
  warm = setTimeout(() => {
    const p = S.pane()
    const list = S.visible(p)
    const around = S.state.quickLook ? [0, 1, -1, 2] : [0]
    for (const d of around) {
      const e = list[p.cursor + d]
      // Never for archive entries: a solid archive decompresses everything up to the entry just to warm it.
      if (!e || kindOf(e) !== 'image' || S.parseArchive(e.path)) continue
      // On a share, nothing is fetched ahead of time: only what's open, or small enough not to matter.
      if (p.remote !== false && !(S.state.quickLook && d === 0) && S.remoteHold(p, e.size)) continue
      imageUrl(e.path)
    }
  }, S.state.quickLook ? 0 : 120)
}, { immediate: true })

function step(d: number) {
  const p = S.pane()
  S.setCursor(p, p.cursor + d, 'move')
  scrollCursorIntoView(p)
}

function openEntry() {
  const p = S.pane()
  close()
  S.open(p, entry.value)
}

// Quick Look lives while Space is held (closing on release is in lib/keys.ts). Swallow Space here so
// auto-repeat or focus inside the dialog (a header button, the viewer...) never activates anything.
function onSpace(e: KeyboardEvent) {
  if (!open.value || e.key !== ' ' || e.ctrlKey || e.metaKey || e.altKey) return
  const t = e.target as HTMLElement
  if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable) return
  e.preventDefault()
  e.stopPropagation()
  // Opened another way (toolbar, palette): a Space press closes it.
  if (!e.repeat && !spaceHeldOpen()) close()
}
onMounted(() => window.addEventListener('keydown', onSpace, true))
onBeforeUnmount(() => window.removeEventListener('keydown', onSpace, true))

function onKey(e: KeyboardEvent) {
  const t = e.target as HTMLElement
  if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable) return
  if (e.ctrlKey || e.metaKey || e.altKey) return
  const k = e.key
  if (k === ' ') return
  if (k === 'ArrowLeft' || k === 'ArrowUp') step(-1)
  else if (k === 'ArrowRight' || k === 'ArrowDown') step(1)
  else if (k === 'Enter') openEntry()
  else return
  e.preventDefault()
  e.stopPropagation()
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay :class="motion" class="ql-overlay fixed inset-0 z-40 bg-overlay" />
      <DialogContent
        :class="motion"
        class="ql-content fixed top-1/2 left-1/2 z-50 flex h-[82vh] w-[80vw] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none"
        @keydown="onKey"
        @open-auto-focus.prevent="($event.currentTarget as HTMLElement | null)?.focus()"
      >
        <template v-if="entry">
          <QuickLookHeader :entry="entry" :pos="pos" @open="openEntry" />
          <div class="min-h-0 flex-1 bg-bg/50">
            <component :is="viewer" v-if="shown" :key="shown.path" :entry="shown" />
            <div v-else-if="pending" class="grid h-full place-items-center"><Icon v-if="open" name="loader" class="size-6 animate-spin text-faint" /></div>
            <div v-else class="flex h-full flex-col items-center justify-center gap-3 text-dim">
              <Icon :name="entry.isDir ? 'folder' : 'file'" class="size-12" />
              <p class="max-w-[70%] truncate text-[13px]">{{ entry.name }}</p>
              <p class="text-[12px]">{{ noPreview }}</p>
            </div>
          </div>
        </template>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.ql-overlay[data-state='open'] { animation: ql-fade-in 90ms ease-out }
.ql-overlay[data-state='closed'] { animation: ql-fade-out 80ms ease-in }
.ql-content[data-state='open'] { animation: ql-zoom-in 120ms ease-out }
.ql-content[data-state='closed'] { animation: ql-zoom-out 80ms ease-in }
.ql-motion-reduced[data-state] { animation-name: ql-fade-in }
.ql-motion-reduced[data-state='closed'] { animation-name: ql-fade-out }
.ql-motion-off[data-state] { animation: none }
@keyframes ql-fade-in { from { opacity: 0 } }
@keyframes ql-fade-out { to { opacity: 0 } }
@keyframes ql-zoom-in { from { opacity: 0; scale: 0.98 } }
@keyframes ql-zoom-out { to { opacity: 0; scale: 0.96 } }
</style>
