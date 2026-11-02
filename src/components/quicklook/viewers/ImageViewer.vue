<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useImageUrl } from '../../../composables/useImageUrl'
import Icon from '../../Icon.vue'
import type { Entry } from '../../../lib/store'

const props = defineProps<{ entry: Entry }>()
const box = ref<HTMLDivElement>()
const src = useImageUrl(() => props.entry.path)
const nat = ref({ w: 0, h: 0 })
const status = ref<'loading' | 'ok' | 'error'>('loading')
const scale = ref(1)
const pos = ref({ x: 0, y: 0 })
const fitted = ref(true)

watch(src, (u) => {
  status.value = u === null ? 'error' : 'loading'
  nat.value = { w: 0, h: 0 }
  fitted.value = true
})

function fitScale() {
  const el = box.value
  if (!el || !nat.value.w) return 1
  return Math.min(1, el.clientWidth / nat.value.w, el.clientHeight / nat.value.h)
}
function center(s: number) {
  const el = box.value!
  scale.value = s
  pos.value = { x: (el.clientWidth - nat.value.w * s) / 2, y: (el.clientHeight - nat.value.h * s) / 2 }
}
function fit() {
  fitted.value = true
  center(fitScale())
}
function onLoad(e: Event) {
  const img = e.target as HTMLImageElement
  nat.value = { w: img.naturalWidth || 512, h: img.naturalHeight || 512 }
  status.value = 'ok'
  fit()
}

function zoomAt(cx: number, cy: number, next: number) {
  const s = Math.min(32, Math.max(0.02, next))
  const k = s / scale.value
  pos.value = { x: cx - (cx - pos.value.x) * k, y: cy - (cy - pos.value.y) * k }
  scale.value = s
  fitted.value = false
}
function onWheel(e: WheelEvent) {
  const r = box.value!.getBoundingClientRect()
  const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015))
  zoomAt(e.clientX - r.left, e.clientY - r.top, scale.value * factor)
}
function onDbl(e: MouseEvent) {
  if (!fitted.value) return fit()
  const r = box.value!.getBoundingClientRect()
  zoomAt(e.clientX - r.left, e.clientY - r.top, 1)
}

let drag: { x: number; y: number } | null = null
function onDown(e: PointerEvent) {
  if (e.button !== 0) return
  drag = { x: e.clientX - pos.value.x, y: e.clientY - pos.value.y }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function onMove(e: PointerEvent) {
  if (drag) pos.value = { x: e.clientX - drag.x, y: e.clientY - drag.y }
}

let ro: ResizeObserver | undefined
onMounted(() => {
  ro = new ResizeObserver(() => fitted.value && status.value === 'ok' && fit())
  ro.observe(box.value!)
})
onBeforeUnmount(() => ro?.disconnect())
</script>

<template>
  <div
    ref="box"
    class="checker relative h-full w-full touch-none overflow-hidden select-none"
    :class="drag ? 'cursor-grabbing' : 'cursor-grab'"
    @wheel.prevent="onWheel"
    @dblclick="onDbl"
    @pointerdown="onDown"
    @pointermove="onMove"
    @pointerup="drag = null"
    @pointercancel="drag = null"
  >
    <img
      v-if="src"
      :key="src"
      :src="src"
      draggable="false"
      class="absolute top-0 left-0 max-w-none origin-top-left"
      :class="status !== 'ok' && 'invisible'"
      :style="{ width: `${nat.w}px`, height: `${nat.h}px`, transform: `translate(${pos.x}px, ${pos.y}px) scale(${scale})`, imageRendering: scale > 2 ? 'pixelated' : 'auto' }"
      @load="onLoad"
      @error="status = 'error'"
    />
    <div v-if="status === 'loading'" class="absolute inset-0 grid place-items-center">
      <Icon name="loader" class="size-6 animate-spin text-faint" />
    </div>
    <div v-else-if="status === 'error'" class="absolute inset-0 grid place-items-center text-center text-[13px] text-dim">
      <div><Icon name="image-off" class="mx-auto mb-2 size-10 text-faint" :stroke-width="1.25" />This image can't be displayed</div>
    </div>
    <div v-else class="pointer-events-none absolute right-3 bottom-3 rounded-md bg-black/60 px-2 py-1 font-mono text-[11px] text-white/85 backdrop-blur">
      {{ nat.w }} × {{ nat.h }} · {{ Math.round(scale * 100) }}%
    </div>
  </div>
</template>

<style scoped>
.checker {
  background-color: var(--color-bg);
  background-image: conic-gradient(var(--color-raised) 25%, transparent 0 50%, var(--color-raised) 0 75%, transparent 0);
  background-size: 20px 20px;
}
</style>
