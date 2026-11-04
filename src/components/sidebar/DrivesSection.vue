<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem } from 'reka-ui'
import type { IconName } from '../../lib/icons'
import Icon from '../Icon.vue'
import UnlockDialog from './UnlockDialog.vue'
import { state, pane, go, transfer } from '../../lib/store'
import { copyMod, fmtSize } from '../../lib/util'
import { drives, ejectable, clickDrive, unmountPlace, refreshDrives } from './drives'
import { openCleanup } from '../cleanup/cleanup'
import { DRIVES } from '../../lib/store'
import { t } from '../../lib/i18n'

const disks = computed(() => state.places.filter((p) => ['disk', 'removable'].includes(p.kind)))
const current = computed(() => pane().path)
const icon = (removable: boolean): IconName => (removable ? 'usb' : 'hard-drive')

function drop(e: DragEvent, dest: string) {
  const data = e.dataTransfer?.getData('application/x-morfa-paths')
  if (data) transfer(JSON.parse(data), dest, copyMod(e) ? 'copy' : 'move')
}

// Cheap refresh: on mount and whenever the window regains focus.
onMounted(() => {
  refreshDrives()
  window.addEventListener('focus', refreshDrives)
})
onUnmounted(() => window.removeEventListener('focus', refreshDrives))
</script>

<template>
  <section>
    <button
      class="flex w-full items-center rounded px-2 pb-1 text-left text-[11px] font-medium uppercase tracking-wider transition hover:text-fg"
      :class="current === DRIVES ? 'text-fg' : 'text-faint'" :title="t('drive.showAll')" @click="go(pane(), DRIVES)"
    >{{ t('drive.title') }}</button>
    <ContextMenuRoot v-for="d in disks" :key="d.path">
    <ContextMenuTrigger as-child>
    <div class="group relative">
      <button
        class="item !h-auto flex-col !items-stretch !gap-1 py-1.5" :class="current === d.path && 'on'" :title="d.path"
        @click="go(pane(), d.path)" @dragover.prevent @drop.prevent="drop($event, d.path)"
      >
        <span class="flex items-center gap-2.5 pr-5">
          <Icon :name="icon(d.kind === 'removable')" class="size-4 shrink-0" /><span class="truncate">{{ d.name }}</span>
        </span>
        <span v-if="d.total" class="h-1 overflow-hidden rounded-full bg-line">
          <span class="block h-full rounded-full"
            :class="d.free / d.total < 0.1 ? 'bg-danger' : 'bg-accent'"
            :style="{ width: `${100 - (d.free / d.total) * 100}%` }" />
        </span>
        <span v-if="d.total" class="text-[11px] text-faint">{{ t('drive.freeOf', { free: fmtSize(d.free), total: fmtSize(d.total) }) }}</span>
        <span v-else-if="d.unavailable" class="text-[11px] text-danger">{{ t('drive.notResponding') }}</span>
      </button>
      <button
        v-if="ejectable(d)" class="absolute top-1.5 right-1.5 grid size-5 place-items-center rounded text-dim hover:bg-line hover:text-fg"
        :class="drives.busy.has(d.path) ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'"
        :title="d.kind === 'removable' ? t('drive.eject') : t('drive.unmount')" :disabled="drives.busy.has(d.path)" @click="unmountPlace(d)"
      >
        <Icon :name="drives.busy.has(d.path) ? 'loader' : 'eject'" class="size-3.5" :class="drives.busy.has(d.path) && 'animate-spin'" />
      </button>
    </div>
    </ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent class="menu">
        <ContextMenuItem class="mi" @select="go(pane(), d.path)"><Icon name="folder-open" />{{ t('shell.open') }}</ContextMenuItem>
        <ContextMenuItem class="mi" @select="openCleanup(d.path)"><Icon name="zap" />{{ t('drive.cleanup') }}</ContextMenuItem>
        <ContextMenuItem v-if="ejectable(d)" class="mi" @select="unmountPlace(d)"><Icon name="eject" />{{ d.kind === 'removable' ? t('drive.eject') : t('drive.unmount') }}</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenuPortal>
    </ContextMenuRoot>
    <button
      v-for="d in drives.list" :key="d.device" class="item opacity-60 hover:opacity-100"
      :title="`${d.device} · ${d.fstype} — ${t(d.encrypted ? 'drive.clickUnlock' : 'drive.clickMount')}`" @click="clickDrive(d)"
    >
      <Icon :name="icon(d.removable)" class="size-4 shrink-0" />
      <span class="truncate">{{ d.name || t('drive.volume', { size: fmtSize(d.size) }) }}</span>
      <span class="ml-auto flex shrink-0 items-center gap-1 text-[11px] text-faint">
        <template v-if="d.name">{{ fmtSize(d.size) }}</template>
        <Icon v-if="drives.busy.has(d.device)" name="loader" class="size-3.5 animate-spin" />
        <Icon v-else-if="d.encrypted" name="lock" class="size-3.5" />
      </span>
    </button>
    <UnlockDialog />
  </section>
</template>

<style scoped>
@reference '../../style.css';
.item {
  @apply flex h-7 w-full items-center gap-2.5 rounded-md px-2 text-left text-[12.5px] text-dim transition hover:bg-raised hover:text-fg;
}
.item.on {
  @apply bg-accent-soft text-fg;
}
</style>
