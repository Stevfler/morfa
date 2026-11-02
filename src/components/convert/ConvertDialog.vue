<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import { convertFileSrc } from '@tauri-apps/api/core'
import Icon from '../Icon.vue'
import FormatPicker from './FormatPicker.vue'
import QualityPicker from './QualityPicker.vue'
import ResizePicker from './ResizePicker.vue'
import OutputPicker from './OutputPicker.vue'
import SizeEstimate from './SizeEstimate.vue'
import EngineBanner from './EngineBanner.vue'
import * as S from '../../lib/store'
import { caps, loadCaps, defaultOpts, runConvert, PRESETS, type ConvertOpts } from '../../lib/convert'
import { basename, load, save } from '../../lib/util'

type Remembered = { format: string; preset: string; quality: number; stripMetadata: boolean }
const KEY = 'orbit.convert.last'
const LOSSLESS = new Set(['png', 'bmp', 'tiff', 'gif'])

const state = S.state
const open = computed({ get: () => !!state.convert, set: (v) => !v && (state.convert = null) })
const paths = computed(() => state.convert?.paths ?? [])
const opts = reactive<ConvertOpts>(defaultOpts('webp'))
const preset = ref('balanced')
const loading = ref(false)

watch(open, async (v) => {
  if (!v) return
  const last = load<Remembered | null>(KEY, null)
  Object.assign(opts, defaultOpts(state.convert?.format ?? last?.format ?? 'webp'))
  if (last) {
    preset.value = last.preset
    if (!state.convert?.format || state.convert.format === last.format) opts.quality = last.quality
    opts.stripMetadata = last.stripMetadata
  } else preset.value = PRESETS.find((p) => p.quality === opts.quality)?.key ?? 'custom'
  if (!caps.value) {
    loading.value = true
    try { await loadCaps() } catch { /* banner covers it */ } finally { loading.value = false }
  }
  const avail = caps.value?.formats ?? []
  if (avail.length && !avail.includes(opts.format)) opts.format = avail.includes('webp') ? 'webp' : avail[0]
}, { immediate: true })

const lossless = computed(() => LOSSLESS.has(opts.format))
const noEngine = computed(() => !loading.value && (!caps.value || caps.value.engine === null))
const title = computed(() => (paths.value.length === 1 ? `Convert ${basename(paths.value[0])}` : `Convert ${paths.value.length} images`))
const thumbs = computed(() => paths.value.slice(0, 6).map((p) => ({ p, src: convertFileSrc(p) })))
const example = computed(() => {
  const n = basename(paths.value[0] ?? 'image.jpg')
  const dot = n.lastIndexOf('.')
  const stem = dot > 0 ? n.slice(0, dot) : n
  return (opts.nameTemplate || '{name}').replace(/\{name\}/g, stem) + '.' + opts.format
})
const canConvert = computed(() => !loading.value && !noEngine.value && paths.value.length > 0 && !!opts.nameTemplate.trim())

function submit() {
  if (!canConvert.value) return
  save(KEY, { format: opts.format, preset: preset.value, quality: opts.quality, stripMetadata: opts.stripMetadata } satisfies Remembered)
  const sources = [...paths.value]
  const o: ConvertOpts = { ...opts, resize: opts.resize ? { ...opts.resize } : null }
  state.convert = null
  void runConvert(sources, o)
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.isComposing && !(e.target instanceof HTMLButtonElement)) { e.preventDefault(); submit() }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[88vh] w-[min(640px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl border border-line bg-panel shadow-2xl outline-none"
        @keydown="onKey"
      >
        <header class="flex items-start gap-4 px-7 pt-6 pb-5">
          <div class="flex shrink-0 -space-x-3">
            <img
              v-for="(t, i) in thumbs.slice(0, 3)" :key="t.p" :src="t.src" alt="" loading="lazy"
              class="size-12 rounded-lg bg-raised object-cover ring-2 ring-panel" :style="{ zIndex: 3 - i }"
            />
          </div>
          <div class="min-w-0 flex-1">
            <DialogTitle class="truncate text-[16px] font-medium">{{ title }}</DialogTitle>
            <DialogDescription class="mt-1 text-[12px] text-faint">
              Originals are never changed — new files are written alongside{{ thumbs.length < paths.length ? ` · ${paths.length - thumbs.length} more not shown` : '' }}.
            </DialogDescription>
          </div>
          <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" aria-label="Close">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </header>

        <div class="min-h-0 flex-1 space-y-7 overflow-y-auto px-7 pb-6">
          <EngineBanner v-if="noEngine" />
          <section>
            <h3 class="sec">Format</h3>
            <FormatPicker v-model="opts.format" />
          </section>
          <section>
            <h3 class="sec">Quality</h3>
            <QualityPicker v-model:quality="opts.quality" v-model:preset="preset" :lossless="lossless" />
          </section>
          <section>
            <h3 class="sec">Size</h3>
            <ResizePicker v-model="opts.resize" />
          </section>
          <section>
            <label class="flex cursor-pointer items-center justify-between gap-4 rounded-lg px-1 py-1">
              <span>
                <span class="block text-[13px] text-fg">Strip metadata</span>
                <span class="block text-[11.5px] text-faint">Removes EXIF, camera details and GPS location</span>
              </span>
              <input v-model="opts.stripMetadata" type="checkbox" role="switch" class="size-4 accent-accent" />
            </label>
          </section>
          <section>
            <h3 class="sec">Save to</h3>
            <OutputPicker v-model:out-dir="opts.outDir" v-model:name-template="opts.nameTemplate" :example="example" />
          </section>
        </div>

        <footer class="flex items-center gap-3 border-t border-line/60 px-7 py-4">
          <SizeEstimate :source="paths[0] ?? null" :opts="opts" :disabled="noEngine || loading" />
          <DialogClose class="ml-auto h-9 rounded-lg px-4 text-[12.5px] text-dim transition hover:bg-raised hover:text-fg">Cancel</DialogClose>
          <button
            class="flex h-9 items-center gap-2 rounded-lg bg-accent px-5 text-[12.5px] font-medium text-on-accent transition hover:brightness-110 disabled:opacity-40"
            :disabled="!canConvert" @click="submit"
          >
            <Icon v-if="loading" name="loader" class="size-3.5 animate-spin" />
            Convert
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
@reference '../../style.css';
.sec {
  @apply mb-3 text-[11px] font-medium tracking-wider text-faint uppercase;
}
</style>
