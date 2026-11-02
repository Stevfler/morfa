<script setup lang="ts">
import { computed } from 'vue'
import type { IconName } from '../lib/icons'
import Icon from './Icon.vue'
import DrivesSection from './sidebar/DrivesSection.vue'
import { state, pane, go, frecencyScore, transfer } from '../lib/store'
import { basename, copyMod, keyLabel } from '../lib/util'
import { DIRS_MIME } from '../composables/useDragDrop'

const ICONS: Record<string, IconName> = {
  home: 'home', desktop: 'monitor', downloads: 'download', documents: 'file-text',
  pictures: 'image', music: 'music', videos: 'video',
}

const folders = computed(() => state.places.filter((p) => !['disk', 'removable'].includes(p.kind)))
const recent = computed(() =>
  Object.keys(state.frecency)
    .filter((p) => !state.bookmarks.includes(p) && !state.places.some((pl) => pl.path === p))
    .sort((a, b) => frecencyScore(b) - frecencyScore(a))
    .slice(0, 6),
)
const current = computed(() => pane().path)

function drop(e: DragEvent, dest: string) {
  const data = e.dataTransfer?.getData('application/x-orbit-paths')
  if (!data) return
  transfer(JSON.parse(data), dest, copyMod(e) ? 'copy' : 'move')
}

function onDropFav(e: DragEvent) {
  const data = e.dataTransfer?.getData(DIRS_MIME)
  if (!data) return
  for (const p of JSON.parse(data) as string[]) if (!state.bookmarks.includes(p)) state.bookmarks.push(p)
}
</script>

<template>
  <aside class="flex w-56 shrink-0 flex-col gap-4 overflow-y-auto border-r border-line bg-panel px-2 py-3">
    <section>
      <h3 class="px-2 pb-1 text-[11px] font-medium uppercase tracking-wider text-faint">Places</h3>
      <button
        v-for="p in folders" :key="p.path"
        class="item" :class="current === p.path && 'on'"
        @click="go(pane(), p.path)" @dragover.prevent @drop.prevent="drop($event, p.path)"
      >
        <Icon :name="ICONS[p.kind]" class="size-4 shrink-0" />{{ p.name }}
      </button>
    </section>

    <section @dragover.prevent @drop.prevent="onDropFav">
      <h3 class="px-2 pb-1 text-[11px] font-medium uppercase tracking-wider text-faint">Favorites</h3>
      <p v-if="!state.bookmarks.length" class="mx-2 rounded-md border border-dashed border-line px-2 py-2 text-[11.5px] leading-snug text-faint">
        Drag folders here or press <kbd>{{ keyLabel('Ctrl+D') }}</kbd>
      </p>
      <button
        v-for="b in state.bookmarks" :key="b"
        class="item group" :class="current === b && 'on'" :title="b"
        @click="go(pane(), b)" @dragover.prevent @drop.prevent.stop="drop($event, b)"
      >
        <Icon name="star" class="size-4 shrink-0 text-warn" /><span class="truncate">{{ basename(b) || b }}</span>
        <Icon name="x" class="ml-auto size-3.5 shrink-0 opacity-0 group-hover:opacity-60 hover:!opacity-100"
           @click.stop="state.bookmarks.splice(state.bookmarks.indexOf(b), 1)" />
      </button>
    </section>

    <section v-if="recent.length">
      <h3 class="px-2 pb-1 text-[11px] font-medium uppercase tracking-wider text-faint">Frequent</h3>
      <button v-for="r in recent" :key="r" class="item" :class="current === r && 'on'" :title="r" @click="go(pane(), r)">
        <Icon name="clock" class="size-4 shrink-0" /><span class="truncate">{{ basename(r) || r }}</span>
      </button>
    </section>

    <DrivesSection />
  </aside>
</template>

<style scoped>
@reference '../style.css';
.item {
  @apply flex h-7 w-full items-center gap-2.5 rounded-md px-2 text-left text-[12.5px] text-dim transition hover:bg-raised hover:text-fg;
}
.item.on {
  @apply bg-accent-soft text-fg;
}
</style>
