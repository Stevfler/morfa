<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { Gauge, ExternalLink, FolderOpen, LoaderCircle } from 'lucide-vue-next'
import * as S from '../lib/store'
import type { Entry } from '../lib/store'
import { fmtDateFull, fmtSize, keyLabel, kindOf, KIND_COLOR, KIND_ICON, KIND_LABEL } from '../lib/util'

const state = S.state
const entry = computed<Entry | undefined>(() => {
  const p = S.pane()
  return S.visible(p)[p.cursor]
})
const kind = computed(() => (entry.value ? kindOf(entry.value) : 'file'))
const src = computed(() => (entry.value ? convertFileSrc(entry.value.path) : ''))

const text = ref<string | null>(null)
const children = ref<Entry[] | null>(null)
const loading = ref(false)
let seq = 0

// Debounced so holding ↓ through a folder doesn't hammer the disk.
let t: ReturnType<typeof setTimeout> | undefined
watch(
  entry,
  (e) => {
    clearTimeout(t)
    text.value = null
    children.value = null
    if (!e) return
    const my = ++seq
    t = setTimeout(async () => {
      loading.value = true
      try {
        if (e.isDir) {
          const list = await invoke<Entry[]>('list_dir', { path: e.path })
          if (my === seq) children.value = list.filter((c) => state.showHidden || !c.hidden)
        } else if (['text', 'code'].includes(kindOf(e)) || (kindOf(e) === 'file' && e.size < 2e6)) {
          const s = await invoke<string | null>('read_text', { path: e.path, max: 64 * 1024 })
          if (my === seq) text.value = s
        }
      } catch {
        /* unreadable — show metadata only */
      } finally {
        if (my === seq) loading.value = false
      }
    }, 70)
  },
  { immediate: true },
)

const childStats = computed(() => {
  if (!children.value) return null
  const dirs = children.value.filter((c) => c.isDir).length
  return { dirs, files: children.value.length - dirs }
})
const size = computed(() => {
  const e = entry.value
  if (!e) return ''
  if (!e.isDir) return fmtSize(e.size)
  const s = state.sizes[e.path]
  return s === 'pending' ? 'calculating…' : typeof s === 'number' ? fmtSize(s) : null
})
</script>

<template>
  <aside class="flex w-80 shrink-0 flex-col overflow-hidden border-l border-line bg-panel">
    <div v-if="!entry" class="grid flex-1 place-items-center p-6 text-center text-[12px] text-faint">
      Select something to preview it here.<br />Press <kbd>{{ keyLabel('Ctrl+I') }}</kbd> to hide this panel.
    </div>
    <template v-else>
      <div class="grid min-h-48 shrink-0 place-items-center overflow-hidden border-b border-line bg-bg/50 p-3" :class="text != null && 'hidden'">
        <img v-if="kind === 'image'" :key="src" :src="src" class="max-h-72 max-w-full rounded object-contain shadow-lg" />
        <component v-else :is="KIND_ICON[kind]" class="size-20" :style="{ color: KIND_COLOR[kind] }" :stroke-width="1" :fill="entry.isDir ? 'currentColor' : 'none'" :fill-opacity="0.15" />
      </div>

      <pre v-if="text != null" class="min-h-0 flex-1 overflow-auto bg-bg/50 p-3 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap break-words text-dim select-text">{{ text || '(empty file)' }}</pre>

      <div class="shrink-0 space-y-3 p-4" :class="text == null && 'flex-1 overflow-y-auto'">
        <div>
          <h2 class="text-[14px] font-medium break-all text-fg">{{ entry.name }}</h2>
          <p class="mt-0.5 text-[11.5px] text-faint">{{ entry.isDir ? 'Folder' : `${KIND_LABEL[kind]}${entry.ext ? ` · .${entry.ext}` : ''}` }}</p>
        </div>
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[12px]">
          <dt class="text-faint">Size</dt>
          <dd class="text-fg">
            <template v-if="size">{{ size }}</template>
            <button v-else class="inline-flex items-center gap-1 text-accent hover:underline" @click="S.computeSizes([entry.path])">
              <Gauge class="size-3.5" />Calculate
            </button>
          </dd>
          <template v-if="childStats">
            <dt class="text-faint">Contains</dt>
            <dd class="text-fg">{{ childStats.dirs }} folders, {{ childStats.files }} files</dd>
          </template>
          <dt class="text-faint">Modified</dt>
          <dd class="text-fg">{{ fmtDateFull(entry.modified) }}</dd>
          <dt class="text-faint">Where</dt>
          <dd class="font-mono text-[11px] break-all text-dim select-text">{{ entry.path }}</dd>
        </dl>

        <ul v-if="children?.length" class="space-y-0.5 border-t border-line pt-3">
          <li v-for="c in children.slice(0, 14)" :key="c.path" class="flex items-center gap-2 truncate text-[12px] text-dim">
            <component :is="KIND_ICON[kindOf(c)]" class="size-3.5 shrink-0" :style="{ color: KIND_COLOR[kindOf(c)] }" />
            <span class="truncate">{{ c.name }}</span>
          </li>
          <li v-if="children.length > 14" class="text-[11.5px] text-faint">+ {{ children.length - 14 }} more</li>
        </ul>

        <div class="flex gap-2 pt-1">
          <button class="btn" @click="S.open(S.pane(), entry)">
            <component :is="entry.isDir ? FolderOpen : ExternalLink" class="size-3.5" />{{ entry.isDir ? 'Open' : 'Open with default app' }}
          </button>
        </div>
        <LoaderCircle v-if="loading" class="size-4 animate-spin text-faint" />
      </div>
    </template>
  </aside>
</template>

<style scoped>
@reference '../style.css';
.btn {
  @apply inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-raised px-2.5 text-[12px] text-fg transition hover:border-accent/60;
}
</style>
