<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useImageUrl } from '../composables/useImageUrl'
import Icon from './Icon.vue'
import * as S from '../lib/store'
import type { Entry } from '../lib/store'
import { fmtDateFull, fmtSize, keyLabel, kindOf, KIND_COLOR, KIND_ICON, KIND_LABEL } from '../lib/util'
import { t } from '../lib/i18n'
import { branchOf } from '../lib/git'

const state = S.state
const entry = computed<Entry | undefined>(() => {
  const p = S.pane()
  return S.visible(p)[p.cursor]
})
const kind = computed(() => (entry.value ? kindOf(entry.value) : 'file'))
/** Temp copy of an image inside an archive, once unpacked. */
const unpackedImage = ref<string | null>(null)
/** On a share, big files and folder contents wait for a click. */
const asked = ref(false)
const held = computed(() => {
  const e = entry.value
  if (!e || asked.value || S.parseArchive(e.path)) return false
  const p = S.pane()
  return e.isDir ? p.remote !== false : S.remoteHold(p, e.size) && (kindOf(e) === 'image' || ['text', 'code'].includes(kindOf(e)))
})
const src = useImageUrl(() => (entry.value && kindOf(entry.value) === 'image' && !held.value ? (S.parseArchive(entry.value.path) ? unpackedImage.value : entry.value.path) : null))

const text = ref<string | null>(null)
const children = ref<Entry[] | null>(null)
const loading = ref(false)
let seq = 0

/** Quick Look closing cancels an unpack it shared with us: try again once if the entry is still current. */
async function unpack(path: string, my: number) {
  try {
    return await S.previewLocal(path)
  } catch (err) {
    if (err !== 'Cancelled' || my !== seq) throw err
    return S.previewLocal(path)
  }
}

// Debounced so holding ↓ through a folder doesn't hammer the disk.
let timer: ReturnType<typeof setTimeout> | undefined
watch(entry, (e, old) => {
  if (e?.path !== old?.path) asked.value = false
}, { flush: 'sync' })
watch(
  [entry, held],
  ([e], [old]) => {
    clearTimeout(timer)
    if (e?.path !== old?.path) S.cancelPreviewUnpack()
    text.value = null
    children.value = null
    unpackedImage.value = null
    if (!e) return
    const my = ++seq
    loading.value = false
    if (held.value) return
    timer = setTimeout(async () => {
      loading.value = true
      try {
        if (e.isDir) {
          const arc = S.parseArchive(e.path)
          const list = await (arc ? invoke<Entry[]>('archive_list', { archive: arc.archive, rel: arc.rel }) : invoke<Entry[]>('list_dir', { path: e.path }))
          if (my === seq) children.value = list.filter((c) => state.showHidden || !c.hidden)
        } else if (S.parseArchive(e.path) && (!S.archiveAutoPreview(e) || kindOf(e) === 'file')) {
          /* not unpacked on cursor movement — metadata only */
        } else if (kindOf(e) === 'image') {
          if (S.parseArchive(e.path)) {
            const local = await unpack(e.path, my)
            if (my === seq) unpackedImage.value = local
          }
        } else if (['text', 'code'].includes(kindOf(e)) || (kindOf(e) === 'file' && e.size < 2e6)) {
          const path = S.parseArchive(e.path) ? await unpack(e.path, my) : e.path
          if (my !== seq) return
          const s = await invoke<string | null>('read_text', { path, max: 64 * 1024 })
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

onBeforeUnmount(() => {
  clearTimeout(timer)
  S.cancelPreviewUnpack()
})

const branch = computed(() => (entry.value?.isDir ? branchOf(entry.value.path) : null))
const childStats = computed(() => {
  if (!children.value) return null
  const dirs = children.value.filter((c) => c.isDir).length
  return { dirs, files: children.value.length - dirs }
})
const size = computed(() => {
  const e = entry.value
  if (!e) return ''
  if (!e.isDir) return fmtSize(e.size)
  if (S.parseArchive(e.path)) return '—' // dir_size can't walk into an archive
  const s = state.sizes[e.path]
  return s === 'pending' ? t('preview.calculating') : typeof s === 'number' ? fmtSize(s) : null
})
</script>

<template>
  <aside class="card flex w-80 shrink-0 flex-col overflow-hidden rounded-xl bg-panel">
    <div v-if="!entry" class="grid flex-1 place-items-center p-6 text-center text-[12px] text-faint">
      {{ t('preview.empty') }}<br />{{ t('preview.press') }} <kbd>{{ keyLabel('Ctrl+I') }}</kbd> {{ t('preview.toHide') }}
    </div>
    <template v-else>
      <div class="grid min-h-48 shrink-0 place-items-center overflow-hidden m-3 mb-0 rounded-lg bg-bg/50 p-3" :class="text != null && 'hidden'">
        <img v-if="kind === 'image' && src" :key="src" :src="src" class="max-h-72 max-w-full rounded object-contain shadow-lg" />
        <Icon v-else :name="KIND_ICON[kind]" class="size-20" :style="{ color: KIND_COLOR[kind] }" :stroke-width="1" :tint="entry.isDir ? 0.15 : 0" />
      </div>

      <pre v-if="text != null" class="min-h-0 flex-1 overflow-auto bg-bg/50 p-3 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap break-words text-dim select-text">{{ text || t('preview.emptyFile') }}</pre>

      <div class="shrink-0 space-y-3 p-4" :class="text == null && 'flex-1 overflow-y-auto'">
        <div>
          <h2 class="text-[14px] font-medium break-all text-fg">{{ entry.name }}</h2>
          <p class="mt-0.5 text-[11.5px] text-faint">{{ entry.isDir ? t('preview.folder') : `${KIND_LABEL[kind]}${entry.ext ? ` · .${entry.ext}` : ''}` }}</p>
        </div>
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[12px]">
          <dt class="text-faint">{{ t('preview.size') }}</dt>
          <dd class="text-fg">
            <template v-if="size">{{ size }}</template>
            <button v-else class="inline-flex items-center gap-1 text-accent hover:underline" @click="S.computeSizes([entry.path])">
              <Icon name="gauge" class="size-3.5" />{{ t('preview.calculate') }}
            </button>
          </dd>
          <template v-if="childStats">
            <dt class="text-faint">{{ t('preview.contains') }}</dt>
            <dd class="text-fg">{{ t('preview.folders', { count: childStats.dirs }) }}, {{ t('preview.files', { count: childStats.files }) }}</dd>
          </template>
          <template v-if="branch">
            <dt class="text-faint">{{ t('preview.branch') }}</dt>
            <dd class="flex min-w-0 items-center gap-1 text-fg"><Icon name="git-branch" class="size-3.5 shrink-0 text-dim" /><span class="break-all select-text">{{ branch }}</span></dd>
          </template>
          <dt class="text-faint">{{ t('preview.modified') }}</dt>
          <dd class="text-fg">{{ fmtDateFull(entry.modified) }}</dd>
          <dt class="text-faint">{{ t('preview.where') }}</dt>
          <dd class="font-mono text-[11px] break-all text-dim select-text">{{ entry.path }}</dd>
        </dl>

        <ul v-if="children?.length" class="space-y-0.5 pt-3">
          <li v-for="c in children.slice(0, 14)" :key="c.path" class="flex items-center gap-2 truncate text-[12px] text-dim">
            <Icon :name="KIND_ICON[kindOf(c)]" class="size-3.5 shrink-0" :style="{ color: KIND_COLOR[kindOf(c)] }" />
            <span class="truncate">{{ c.name }}</span>
          </li>
          <li v-if="children.length > 14" class="text-[11.5px] text-faint">{{ t('preview.more', { count: children.length - 14 }) }}</li>
        </ul>

        <button v-if="held" class="btn" @click="asked = true">
          <Icon name="eye" class="size-3.5 shrink-0" />{{ entry.isDir ? t('preview.showContents') : t('preview.loadRemote') }}
        </button>

        <div class="flex gap-2 pt-1">
          <button class="btn min-w-0 max-w-full" :title="entry.isDir ? t('shell.open') : t('shell.openDefault')" @click="S.open(S.pane(), entry)">
            <Icon :name="entry.isDir ? 'folder-open' : 'external-link'" class="size-3.5 shrink-0" />
            <span class="truncate">{{ entry.isDir ? t('shell.open') : t('shell.openDefault') }}</span>
          </button>
        </div>
        <Icon name="loader" v-if="loading" class="size-4 animate-spin text-faint" />
      </div>
    </template>
  </aside>
</template>

<style scoped>
@reference '../style.css';
.btn {
  @apply inline-flex h-7 items-center gap-1.5 rounded-md bg-raised px-2.5 text-[12px] text-fg transition hover:bg-accent-soft/40;
}
</style>
