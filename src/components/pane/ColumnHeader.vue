<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../Icon.vue'
import type { Pane, SortKey } from '../../lib/store'

const props = defineProps<{ pane: Pane }>()
const p = props.pane
const searching = computed(() => !!p.search)
const cols = computed(() => [['name', searching.value ? 'Name · location' : 'Name'], ['size', 'Size'], ['modified', 'Modified'], ['ext', 'Kind']] as [SortKey, string][])

function sortBy(key: SortKey) {
  p.sort = p.sort.key === key ? { key, asc: !p.sort.asc } : { key, asc: key === 'name' || key === 'ext' }
}
</script>

<template>
  <div class="file-cols grid h-7 shrink-0 items-center px-3 text-[11.5px] text-faint">
    <button v-for="[k, label] in cols"
      :key="k" class="flex items-center gap-1 hover:text-fg" :class="k !== 'name' && 'justify-end'" :disabled="searching" @click="sortBy(k)"
    >
      {{ label }}
      <Icon name="arrow-down" v-if="p.sort.key === k && !p.filter && !searching" class="size-3 transition" :class="p.sort.asc && 'rotate-180'" />
    </button>
  </div>
</template>
