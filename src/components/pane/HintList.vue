<script setup lang="ts">
import { computed } from 'vue'
import type { Pane } from '../../lib/store'
import { prettyKeys } from '../../lib/commands'
import { hintsFor } from '../../lib/hints'

const props = defineProps<{ pane: Pane; split: boolean }>()
const hint = computed(() => hintsFor(props.pane, props.split))
</script>

<template>
  <span class="flex items-center gap-3 overflow-hidden whitespace-nowrap">
    <span v-for="[k, t] in hint" :key="k" class="flex items-center gap-1"><kbd v-for="kk in prettyKeys(k)" :key="kk">{{ kk }}</kbd>{{ t }}</span>
  </span>
</template>
