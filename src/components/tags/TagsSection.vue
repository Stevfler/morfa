<script setup lang="ts">
import { computed, onMounted } from 'vue'
import Icon from '../Icon.vue'
import { pane } from '../../lib/store'
import { COLORS, allTags, colorOf, refreshAllTags, showTag } from '../../lib/tags'

onMounted(refreshAllTags)
// Colours first in Finder order, then custom tags alphabetically.
const list = computed(() => {
  const used = allTags.value
  const colors = COLORS.map((c) => c.name).filter((t) => used[t])
  const custom = Object.keys(used).filter((t) => !colorOf(t)).sort((a, b) => a.localeCompare(b))
  return [...colors, ...custom]
})
const current = computed(() => pane().search?.query)
</script>

<template>
  <section v-if="list.length">
    <h3 class="px-2 pb-1 text-[11px] font-medium uppercase tracking-wider text-faint">Tags</h3>
    <button
      v-for="t in list" :key="t"
      class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12.5px] text-dim transition hover:bg-raised hover:text-fg"
      :class="current === `#${t}` && 'bg-accent-soft text-fg'" @click="showTag(pane(), t)"
    >
      <span v-if="colorOf(t)" class="mx-1 size-2.5 shrink-0 rounded-full" :style="{ background: colorOf(t) }" />
      <Icon v-else name="tag" class="size-4 shrink-0" />
      <span class="truncate">{{ t }}</span>
      <span class="ml-auto text-[11px] text-faint">{{ allTags[t] }}</span>
    </button>
  </section>
</template>
