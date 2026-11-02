<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import Icon from '../Icon.vue'
import DriveTile from './DriveTile.vue'
import { state } from '../../lib/store'
import type { Pane } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import { drives, clickDrive, refreshDrives } from '../sidebar/drives'
import { t } from '../../lib/i18n'

defineProps<{ pane: Pane }>()
const disks = computed(() => state.places.filter((p) => ['disk', 'removable'].includes(p.kind)))
const grid = ref<HTMLElement>()

onMounted(() => {
  refreshDrives()
  window.addEventListener('focus', refreshDrives)
  grid.value?.querySelector<HTMLElement>('[data-tile]')?.focus()
})
onUnmounted(() => window.removeEventListener('focus', refreshDrives))

// Arrow keys move between tiles; Enter/Space activate the focused button natively.
function onKey(e: KeyboardEvent) {
  const tiles = [...(grid.value?.querySelectorAll<HTMLElement>('[data-tile]') ?? [])]
  const i = tiles.indexOf(document.activeElement as HTMLElement)
  if (e.key === 'Enter' || e.key === ' ') return e.stopPropagation()
  const top = tiles[0]?.getBoundingClientRect().top
  const cols = Math.max(1, tiles.filter((t) => t.getBoundingClientRect().top === top).length)
  const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -cols, ArrowDown: cols }[e.key]
  if (!step) return
  e.preventDefault()
  e.stopPropagation()
  const next = i < 0 ? 0 : Math.min(tiles.length - 1, Math.max(0, i + step))
  tiles[next]?.focus()
}
</script>

<template>
  <div class="min-h-0 flex-1 overflow-y-auto p-5">
    <h2 class="mb-4 text-[13px] font-medium text-dim">{{ t('drive.title') }}</h2>
    <div ref="grid" class="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3" @keydown="onKey">
      <DriveTile v-for="d in disks" :key="d.path" :place="d" :pane="pane" />
      <div v-for="d in drives.list" :key="d.device" class="relative rounded-xl border border-dashed border-line/60 opacity-60 transition hover:opacity-100">
        <button data-tile class="flex w-full flex-col gap-2 rounded-xl p-4 text-left outline-none focus-visible:ring-2 focus-visible:ring-accent"
          :title="`${d.device} · ${d.fstype}`" @click="clickDrive(d)">
          <span class="flex items-start gap-3">
            <Icon :name="d.removable ? 'usb' : 'hard-drive'" class="size-9 shrink-0 text-faint" />
            <span class="min-w-0">
              <span class="block truncate text-[14px] font-medium text-fg">{{ d.name || t('drive.volume', { size: fmtSize(d.size) }) }}</span>
              <span class="block truncate font-mono text-[11.5px] text-faint">{{ d.device }}</span>
            </span>
          </span>
          <span class="flex items-center gap-2 text-[12px] text-dim">
            {{ fmtSize(d.size) }} · {{ t('drive.notMounted') }}
            <span v-if="d.fstype" class="rounded bg-raised px-1.5 py-0.5 font-mono text-[10.5px] uppercase text-faint">{{ d.fstype }}</span>
            <span class="ml-auto flex items-center gap-1 rounded-md bg-raised px-2 py-1 text-fg">
              <Icon :name="drives.busy.has(d.device) ? 'loader' : d.encrypted ? 'lock' : 'hard-drive'" class="size-3.5" :class="drives.busy.has(d.device) && 'animate-spin'" />
              {{ d.encrypted ? t('unlock.unlock') : t('drive.mount') }}
            </span>
          </span>
        </button>
      </div>
    </div>
    <p v-if="!disks.length && !drives.list.length" class="text-[12.5px] text-faint">{{ t('drive.none') }}</p>
  </div>
</template>
