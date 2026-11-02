<script setup lang="ts">
import { computed, nextTick, onMounted, onBeforeUnmount, ref, watch } from 'vue'
import * as S from '../../lib/store'
import { primaryMod } from '../../lib/util'
import type { Pane } from '../../lib/store'
import type { DragDrop } from '../../composables/useDragDrop'
import FileRow from './FileRow.vue'
import EmptyState from './EmptyState.vue'

const props = defineProps<{ pane: Pane; active: boolean; dnd: DragDrop }>()
const emit = defineEmits<{ itemMenu: [] }>()
const p = props.pane
const state = S.state
const { dropTarget, dragStart, dragOver, dragLeave, drop } = props.dnd

const ROW = 30
const list = computed(() => S.visible(p))
const scroller = ref<HTMLElement>()
const scrollTop = ref(0)
const viewH = ref(600)
const range = computed(() => {
  const start = Math.max(0, Math.floor(scrollTop.value / ROW) - 8)
  const end = Math.min(list.value.length, Math.ceil((scrollTop.value + viewH.value) / ROW) + 8)
  return { start, end }
})
const rows = computed(() => list.value.slice(range.value.start, range.value.end).map((e, k) => ({ e, i: range.value.start + k })))

function revealCursor() {
  const el = scroller.value
  if (!el) return
  const top = p.cursor * ROW
  if (top < el.scrollTop) el.scrollTop = top
  else if (top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW - el.clientHeight
}

let ro: ResizeObserver | undefined
onMounted(() => {
  ro = new ResizeObserver(() => (viewH.value = scroller.value?.clientHeight ?? 600))
  if (scroller.value) ro.observe(scroller.value)
  scroller.value?.addEventListener('reveal-cursor', revealCursor)
})
onBeforeUnmount(() => ro?.disconnect())
watch(() => p.path, () => scroller.value && (scroller.value.scrollTop = 0))
watch(() => p.entries, () => nextTick(revealCursor))
// Scroll the row being renamed into view; FileRow focuses its own input.
watch(() => state.renaming, (path) => {
  if (!path) return
  const i = list.value.findIndex((e) => e.path === path)
  if (i >= 0) {
    p.cursor = i
    revealCursor()
  }
})

// Plain mousedown on an already-selected row keeps the selection so it can be
// dragged as a whole; it collapses to that row on click if no drag happened.
let pendingCollapse = -1
function pressRow(ev: MouseEvent, i: number) {
  pendingCollapse = -1
  if (ev.shiftKey) S.setCursor(p, i, 'extend')
  else if (primaryMod(ev)) S.setCursor(p, i, 'toggle')
  else if (p.selected.size > 1 && p.selected.has(list.value[i].path)) {
    p.cursor = i
    pendingCollapse = i
  } else S.setCursor(p, i, 'move')
}
function clickRow(i: number) {
  if (pendingCollapse === i) S.setCursor(p, i, 'move')
  pendingCollapse = -1
}
function contextRow(i: number) {
  const e = list.value[i]
  if (!p.selected.has(e.path)) S.setCursor(p, i, 'move')
  else p.cursor = i
  emit('itemMenu')
}
</script>

<template>
  <div
    ref="scroller" :data-pane="p.id"
    class="relative min-h-0 flex-1 overflow-y-auto outline-none"
    :class="dropTarget === p.path && 'bg-accent-soft/30'"
    @scroll="scrollTop = ($event.target as HTMLElement).scrollTop"
    @click.self="p.selected.clear()"
    @dragover="dragOver($event, p.path)" @dragleave.self="dragLeave" @drop="drop($event, p.path)"
  >
    <div :style="{ height: `${list.length * ROW}px` }" class="relative" @click.self="p.selected.clear()">
      <FileRow
        v-for="{ e, i } in rows" :key="e.path"
        :pane="p" :e="e" :i="i" :row="ROW" :active="active" :drop-target="dropTarget"
        @mousedown.left="pressRow($event, i)"
        @click.left="clickRow(i)"
        @dblclick="S.open(p, e)"
        @contextmenu="contextRow(i)"
        @dragstart="pendingCollapse = -1; dragStart($event, e, list)"
        @dragover.stop="e.isDir ? dragOver($event, e.path) : undefined"
        @dragleave="dragLeave"
        @drop.stop="e.isDir ? drop($event, e.path) : undefined"
      />
    </div>
    <EmptyState v-if="!list.length && !p.loading" :pane="p" />
  </div>
</template>
