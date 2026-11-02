<script setup lang="ts">
import { computed, defineAsyncComponent, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent } from 'reka-ui'
import * as S from '../../lib/store'
import type { Entry } from '../../lib/store'
import { canQuickLook, kindOf, type Kind } from '../../lib/util'
import { scrollCursorIntoView } from '../../lib/keys'
import QuickLookHeader from './QuickLookHeader.vue'

const Image = defineAsyncComponent(() => import('./viewers/ImageViewer.vue'))
const Text = defineAsyncComponent(() => import('./viewers/TextViewer.vue'))
const VIEWERS: Partial<Record<Kind, unknown>> = { image: Image, text: Text, code: Text }

const open = computed({
  get: () => S.state.quickLook && canQuickLook(entry.value),
  set: (v) => (S.state.quickLook = v),
})
const entry = computed<Entry | undefined>(() => {
  const p = S.pane()
  return S.visible(p)[p.cursor]
})
const viewer = computed(() => (entry.value ? VIEWERS[kindOf(entry.value)] : null))
const pos = computed(() => `${S.pane().cursor + 1} / ${S.visible(S.pane()).length}`)

// Folder emptied or navigated away under us: close rather than show nothing.
watch(entry, (e) => !e && (S.state.quickLook = false))

function step(d: number) {
  const p = S.pane()
  S.setCursor(p, p.cursor + d, 'move')
  scrollCursorIntoView(p)
}

function openEntry() {
  const p = S.pane()
  S.state.quickLook = false
  S.open(p, entry.value)
}

function onKey(e: KeyboardEvent) {
  const t = e.target as HTMLElement
  if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable) return
  if (e.ctrlKey || e.metaKey || e.altKey) return
  const k = e.key
  if (k === ' ' && t.closest('button')) return
  if (k === ' ') S.state.quickLook = false
  else if (k === 'ArrowLeft' || k === 'ArrowUp') step(-1)
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
      <DialogOverlay class="ql-overlay fixed inset-0 z-40 bg-black/55 backdrop-blur-sm" />
      <DialogContent
        class="ql-content fixed top-1/2 left-1/2 z-50 flex h-[82vh] w-[80vw] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border border-line bg-panel shadow-2xl outline-none"
        @keydown="onKey"
        @open-auto-focus.prevent="($event.currentTarget as HTMLElement | null)?.focus()"
      >
        <template v-if="entry">
          <QuickLookHeader :entry="entry" :pos="pos" @open="openEntry" />
          <div class="min-h-0 flex-1 bg-bg/50">
            <component :is="viewer" :key="entry.path" :entry="entry" />
          </div>
        </template>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.ql-overlay[data-state='open'] { animation: ql-fade-in 160ms ease-out }
.ql-overlay[data-state='closed'] { animation: ql-fade-out 120ms ease-in }
.ql-content[data-state='open'] { animation: ql-zoom-in 180ms cubic-bezier(0.2, 0.9, 0.3, 1.1) }
.ql-content[data-state='closed'] { animation: ql-zoom-out 120ms ease-in }
@keyframes ql-fade-in { from { opacity: 0 } }
@keyframes ql-fade-out { to { opacity: 0 } }
@keyframes ql-zoom-in { from { opacity: 0; scale: 0.94 } }
@keyframes ql-zoom-out { to { opacity: 0; scale: 0.96 } }
</style>
