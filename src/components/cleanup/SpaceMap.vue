<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../Icon.vue'
import { fmtSize } from '../../lib/util'
import { cleanup, drill, type SpaceNode } from './cleanup'

const parent = computed(() => cleanup.crumbs[cleanup.crumbs.length - 1])
const max = computed(() => Math.max(1, ...cleanup.children.map((c) => c.size)))
const pct = (n: SpaceNode) => (parent.value?.size ? (n.size / parent.value.size) * 100 : 0)
const icon = (n: SpaceNode) => (n.kind === 'dir' ? 'folder' : n.kind === 'file' ? 'file' : 'shapes')
</script>

<template>
  <section class="flex min-h-0 flex-col" aria-label="Space map">
    <nav class="flex min-h-9 flex-wrap items-center gap-0.5 border-b border-line px-3 py-1 text-[12px]" aria-label="Folder path">
      <template v-for="(c, i) in cleanup.crumbs" :key="c.path">
        <Icon v-if="i" name="chevron-right" class="size-3 text-dim" />
        <button
          class="max-w-48 truncate rounded px-1.5 py-0.5 outline-none hover:bg-raised focus-visible:ring-2 focus-visible:ring-accent/60"
          :class="i === cleanup.crumbs.length - 1 ? 'text-fg' : 'text-dim'"
          @click="drill(c, i)"
        >{{ i ? c.name : cleanup.name }}</button>
      </template>
      <span v-if="parent" class="ml-auto pl-2 text-dim tabular-nums">{{ fmtSize(parent.size) }}</span>
    </nav>
    <ul class="min-h-0 flex-1 overflow-y-auto p-2">
      <li v-for="n in cleanup.children" :key="n.kind + n.path">
        <button
          class="group relative flex h-8 w-full items-center gap-2 overflow-hidden rounded-md px-2 text-left text-[12.5px] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60"
          :class="n.kind === 'dir' ? 'hover:bg-raised' : 'cursor-default'"
          :aria-disabled="n.kind !== 'dir'"
          :title="n.path"
          @click="drill(n)"
        >
          <span class="absolute inset-y-1 left-0 rounded-r bg-accent-soft transition-[width]" :style="{ width: (n.size / max) * 100 + '%' }" aria-hidden="true" />
          <Icon :name="icon(n)" class="relative size-4 shrink-0" :class="n.kind === 'dir' ? 'text-[var(--kind-folder)]' : 'text-dim'" />
          <span class="relative min-w-0 flex-1 truncate" :class="n.kind === 'rest' && 'text-dim italic'">{{ n.name }}</span>
          <span class="relative w-12 text-right text-[11px] text-dim tabular-nums">{{ pct(n).toFixed(pct(n) < 10 ? 1 : 0) }}%</span>
          <span class="relative w-16 text-right tabular-nums">{{ fmtSize(n.size) }}</span>
        </button>
      </li>
      <li v-if="!cleanup.children.length" class="p-6 text-center text-[12.5px] text-dim">Empty</li>
    </ul>
  </section>
</template>
