<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import FormatPicker from './FormatPicker.vue'
import QualityPicker from './QualityPicker.vue'
import ResizePicker from './ResizePicker.vue'
import OutputPicker from './OutputPicker.vue'
import SizeEstimate from './SizeEstimate.vue'
import EngineBanner from './EngineBanner.vue'
import ConvertHeader from './ConvertHeader.vue'
import * as S from '../../lib/store'
import { caps, loadCaps, defaultOpts, runConvert, targetsFor, missingEngine, isImage, isImageFormat, PRESETS, type ConvertOpts } from '../../lib/convert'
import { basename, load, save } from '../../lib/util'
import { t } from '../../lib/i18n'

type Remembered = { format: string; preset: string; quality: number; stripMetadata: boolean }
const KEY = 'orbit.convert.last'
const LOSSLESS = new Set(['png', 'bmp', 'tiff', 'gif'])

const state = S.state
const open = computed({ get: () => !!state.convert, set: (v) => !v && (state.convert = null) })
const paths = computed(() => state.convert?.paths ?? [])
const opts = reactive<ConvertOpts>(defaultOpts('webp'))
const preset = ref('balanced')
const loading = ref(false)
const targets = ref<string[] | null>(null)

watch(open, async (v) => {
  if (!v) return
  const last = load<Remembered | null>(KEY, null)
  Object.assign(opts, defaultOpts(state.convert?.format ?? last?.format ?? 'webp'))
  if (last) {
    preset.value = last.preset
    if (!state.convert?.format || state.convert.format === last.format) opts.quality = last.quality
    opts.stripMetadata = last.stripMetadata
  } else preset.value = PRESETS.find((p) => p.quality === opts.quality)?.key ?? 'custom'
  targets.value = null
  loading.value = true
  const forPaths = paths.value
  const [, list] = await Promise.all([
    caps.value ? null : loadCaps().catch(() => null), // banner covers a failure
    targetsFor(forPaths).catch(() => [] as string[]),
  ])
  if (forPaths !== paths.value) return
  loading.value = false
  targets.value = list
  if (list.length && !list.includes(opts.format)) opts.format = list[0]
}, { immediate: true })

const lossless = computed(() => LOSSLESS.has(opts.format))
// A format the backend offers is convertible here; only explain a gap otherwise.
const engine = computed(() => (loading.value || targets.value?.includes(opts.format) ? null : missingEngine(paths.value, opts.format, caps.value)))
const noEngine = computed(() => !!engine.value)
const allImages = computed(() => paths.value.every(isImage))
// Raster output: quality applies (pdf→jpg too); resize/metadata/estimate only for image sources.
const imageOut = computed(() => isImageFormat(opts.format))
const imageIn = computed(() => imageOut.value && allImages.value)
const perPage = computed(() => imageOut.value && paths.value.some((p) => p.toLowerCase().endsWith('.pdf')))
const example = computed(() => {
  const n = basename(paths.value[0] ?? 'image.jpg')
  const dot = n.lastIndexOf('.')
  const stem = dot > 0 ? n.slice(0, dot) : n
  return (opts.nameTemplate || '{name}').replace(/\{name\}/g, stem) + '.' + opts.format
})
const canConvert = computed(() => !loading.value && !noEngine.value && !!targets.value?.includes(opts.format) && paths.value.length > 0 && !!opts.nameTemplate.trim())

function submit() {
  if (!canConvert.value) return
  save(KEY, { format: opts.format, preset: preset.value, quality: opts.quality, stripMetadata: opts.stripMetadata } satisfies Remembered)
  const sources = [...paths.value]
  const o: ConvertOpts = imageIn.value ? { ...opts, resize: opts.resize ? { ...opts.resize } : null } : { ...opts, resize: null, stripMetadata: false }
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
        <ConvertHeader :paths="paths" />

        <div class="min-h-0 flex-1 space-y-7 overflow-y-auto px-7 pb-6">
          <EngineBanner v-if="engine" :engine="engine" />
          <section>
            <h3 class="sec">{{ t('convert.format') }}</h3>
            <FormatPicker v-model="opts.format" :formats="targets" />
            <p v-if="perPage" class="mt-2 text-[12px] text-faint">{{ t('convert.perPage') }}</p>
          </section>
          <section v-if="imageOut">
            <h3 class="sec">{{ t('convert.quality') }}</h3>
            <QualityPicker v-model:quality="opts.quality" v-model:preset="preset" :lossless="lossless" />
          </section>
          <section v-if="imageIn">
            <h3 class="sec">{{ t('convert.size') }}</h3>
            <ResizePicker v-model="opts.resize" />
          </section>
          <section v-if="imageIn">
            <label class="flex cursor-pointer items-center justify-between gap-4 rounded-lg px-1 py-1">
              <span>
                <span class="block text-[13px] text-fg">{{ t('convert.strip') }}</span>
                <span class="block text-[11.5px] text-faint">{{ t('convert.stripDesc') }}</span>
              </span>
              <input v-model="opts.stripMetadata" type="checkbox" role="switch" class="size-4 accent-accent" />
            </label>
          </section>
          <section>
            <h3 class="sec">{{ t('convert.saveTo') }}</h3>
            <OutputPicker v-model:out-dir="opts.outDir" v-model:name-template="opts.nameTemplate" :example="example" />
          </section>
        </div>

        <footer class="flex items-center gap-3 border-t border-line/60 px-7 py-4">
          <SizeEstimate :source="imageIn ? paths[0] ?? null : null" :opts="opts" :disabled="noEngine || loading" />
          <DialogClose class="ml-auto h-9 rounded-lg px-4 text-[12.5px] text-dim transition hover:bg-raised hover:text-fg">{{ t('common.cancel') }}</DialogClose>
          <button
            class="flex h-9 items-center gap-2 rounded-lg bg-accent px-5 text-[12.5px] font-medium text-on-accent transition hover:brightness-110 disabled:opacity-40"
            :disabled="!canConvert" @click="submit"
          >
            <Icon v-if="loading" name="loader" class="size-3.5 animate-spin" />
            {{ t('convert.convert') }}
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
