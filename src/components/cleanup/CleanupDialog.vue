<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import SpaceMap from './SpaceMap.vue'
import CategoryCard from './CategoryCard.vue'
import { fmtSize } from '../../lib/util'
import { state } from '../../lib/store'
import { cleanup, scan, cancelScan, close, removeSelected, selectedBytes } from './cleanup'

const open = computed({ get: () => cleanup.open, set: (v) => !v && close() })
const now = ref(performance.now())
let timer = 0
watch(() => cleanup.scanning, (s) => {
  clearInterval(timer)
  if (s) timer = window.setInterval(() => (now.value = performance.now()), 250)
}, { immediate: true })
onBeforeUnmount(() => clearInterval(timer))
const elapsed = computed(() => {
  const ms = cleanup.scanning ? now.value - cleanup.started : cleanup.report?.elapsed_ms ?? 0
  return ms < 60_000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`
})
const count = computed(() => cleanup.selected.size)

function deleteForever() {
  const n = count.value, size = fmtSize(selectedBytes())
  state.confirm = {
    title: `Permanently delete ${n} item${n === 1 ? '' : 's'}?`,
    desc: `${size} will be erased without going to Trash. This can't be undone.`,
    ok: 'Delete permanently', danger: true,
    run: () => void removeSelected(true),
  }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-40 flex h-[85vh] w-[90vw] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none">
        <header class="flex h-12 shrink-0 items-center gap-3 px-4">
          <Icon name="broom" class="size-4 text-accent" />
          <DialogTitle class="truncate text-[14px] font-medium">Clean up — {{ cleanup.name }}</DialogTitle>
          <DialogDescription class="sr-only">Space used on {{ cleanup.root }} and items that can be cleaned up.</DialogDescription>
          <div class="ml-auto flex items-center gap-3 text-[12px] text-dim tabular-nums" aria-live="polite">
            <Icon v-if="cleanup.scanning" name="loader" class="size-3.5 animate-spin" />
            <span>{{ cleanup.files.toLocaleString() }} files</span>
            <span>{{ fmtSize(cleanup.bytes) }}</span>
            <span>{{ elapsed }}</span>
            <button v-if="cleanup.scanning" class="h-7 rounded-full bg-raised px-3 text-fg hover:bg-accent-soft" @click="cancelScan">Cancel</button>
            <button v-else class="flex h-7 items-center gap-1.5 rounded-full bg-raised px-3 text-fg hover:bg-accent-soft" @click="scan">
              <Icon name="refresh" class="size-3.5" /> Rescan
            </button>
          </div>
          <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" aria-label="Close">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </header>

        <div v-if="cleanup.report" class="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
          <SpaceMap class="min-h-0 rounded-lg" />
          <section class="min-h-0 space-y-2 overflow-y-auto p-3" aria-label="Clean-up suggestions">
            <CategoryCard v-for="c in cleanup.report.categories" :key="c.id" :cat="c" />
          </section>
        </div>
        <div v-else class="grid flex-1 place-items-center text-[13px] text-dim">
          <p v-if="cleanup.scanning">Measuring what takes space on {{ cleanup.name }}…</p>
          <p v-else>{{ cleanup.error || 'No results.' }}</p>
        </div>

        <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
          <span class="text-dim">Selected: <span class="text-fg tabular-nums">{{ count }} item{{ count === 1 ? '' : 's' }}, {{ fmtSize(selectedBytes()) || '0 B' }}</span></span>
          <button class="ml-auto h-8 rounded-md px-3 text-danger hover:bg-danger/10 disabled:opacity-40" :disabled="!count || cleanup.busy" @click="deleteForever">Delete permanently…</button>
          <button class="flex h-8 items-center gap-1.5 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110 disabled:opacity-40" :disabled="!count || cleanup.busy" @click="removeSelected(false)">
            <Icon name="trash" class="size-3.5" /> Move to Trash
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
