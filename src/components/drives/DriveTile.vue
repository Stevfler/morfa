<script setup lang="ts">
import { computed } from 'vue'
import {
  ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator,
} from 'reka-ui'
import type { IconName } from '../../lib/icons'
import Icon from '../Icon.vue'
import * as S from '../../lib/store'
import type { Pane, Place } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import { drives, ejectable, unmountPlace } from '../sidebar/drives'
import { openCleanup } from '../cleanup/cleanup'

const props = defineProps<{ place: Place; pane: Pane }>()
const d = computed(() => props.place)
const used = computed(() => (d.value.total ? 1 - d.value.free / d.value.total : 0))
const isSystem = computed(() => d.value.path === '/' || /^[A-Za-z]:\\?$/.test(d.value.path))
const icon = computed<IconName>(() => (d.value.removable || d.value.kind === 'removable' ? 'usb' : isSystem.value ? 'monitor' : 'hard-drive'))
const bar = computed(() => (used.value > 0.95 ? 'bg-danger' : used.value > 0.85 ? 'bg-warn' : 'bg-accent'))
const busy = computed(() => drives.busy.has(d.value.path))

function openOther() {
  const o = S.otherPane()
  if (o) S.go(o, d.value.path)
}
async function copyPath() {
  await navigator.clipboard.writeText(d.value.path)
  S.toast({ title: 'Path copied', desc: d.value.path, tone: 'info' }, 2000)
}
</script>

<template>
  <ContextMenuRoot>
    <ContextMenuTrigger as-child>
      <div class="tile group relative">
        <button data-tile class="flex w-full flex-col gap-2 rounded-xl p-4 text-left outline-none" :title="d.path" @click="S.go(pane, d.path)">
          <span class="flex items-start gap-3 pr-8">
            <Icon :name="icon" class="size-9 shrink-0 text-accent" />
            <span class="min-w-0">
              <span class="block truncate text-[14px] font-medium text-fg">{{ d.name }}</span>
              <span class="block truncate font-mono text-[11.5px] text-faint">{{ d.path }}</span>
            </span>
          </span>
          <template v-if="d.total">
            <span class="h-2 overflow-hidden rounded-full bg-line">
              <span class="block h-full rounded-full transition-[width]" :class="bar" :style="{ width: `${used * 100}%` }" />
            </span>
            <span class="flex items-center gap-2 text-[12px] text-dim">
              {{ fmtSize(d.free) }} free of {{ fmtSize(d.total) }}
              <span v-if="d.fs" class="ml-auto rounded bg-raised px-1.5 py-0.5 font-mono text-[10.5px] uppercase text-faint">{{ d.fs }}</span>
            </span>
          </template>
        </button>
        <button
          v-if="ejectable(d)" class="absolute top-3 right-3 grid size-7 place-items-center rounded-md text-dim hover:bg-line hover:text-fg"
          :title="d.kind === 'removable' ? 'Eject' : 'Unmount'" :disabled="busy" @click="unmountPlace(d)"
        >
          <Icon :name="busy ? 'loader' : 'eject'" class="size-4" :class="busy && 'animate-spin'" />
        </button>
      </div>
    </ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent class="menu">
        <ContextMenuItem class="mi" @select="S.go(pane, d.path)"><Icon name="folder-open" />Open<span>Enter</span></ContextMenuItem>
        <ContextMenuItem v-if="S.otherPane()" class="mi" @select="openOther"><Icon name="columns" />Open in other pane</ContextMenuItem>
        <ContextMenuItem class="mi" @select="openCleanup(d.path)"><Icon name="zap" />Clean up…</ContextMenuItem>
        <ContextMenuItem class="mi" @select="copyPath"><Icon name="link" />Copy path</ContextMenuItem>
        <template v-if="ejectable(d)">
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi" :disabled="busy" @select="unmountPlace(d)"><Icon name="eject" />{{ d.kind === 'removable' ? 'Eject' : 'Unmount' }}</ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>

<style scoped>
@reference '../../style.css';
.tile {
  @apply rounded-xl border border-line bg-panel transition hover:border-accent/50 hover:bg-raised has-[[data-tile]:focus-visible]:ring-2 has-[[data-tile]:focus-visible]:ring-accent;
}
</style>
