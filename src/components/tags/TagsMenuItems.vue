<script setup lang="ts">
import { computed } from 'vue'
import { ContextMenuItem } from 'reka-ui'
import Icon from '../Icon.vue'
import { COLORS, editing, tagsOf, toggleTag } from '../../lib/tags'

const props = defineProps<{ paths: string[] }>()
const has = (tag: string) => props.paths.length > 0 && props.paths.every((p) => tagsOf(p).includes(tag))
const label = computed(() => (props.paths.length > 1 ? `Tags for ${props.paths.length} items…` : 'Tags…'))
</script>

<template>
  <div class="flex items-center gap-1.5 px-2.5 py-1.5">
    <ContextMenuItem
      v-for="c in COLORS" :key="c.name" :title="c.name" :aria-label="c.name"
      class="grid size-4.5 cursor-default place-items-center rounded-full outline-none ring-offset-1 ring-offset-panel data-highlighted:ring-2 data-highlighted:ring-fg/40"
      :style="{ background: c.css }" @select="toggleTag(paths, c.name)"
    >
      <Icon v-if="has(c.name)" name="check" class="size-3 text-white" />
    </ContextMenuItem>
  </div>
  <ContextMenuItem class="mi" @select="editing = paths"><Icon name="tag" />{{ label }}</ContextMenuItem>
</template>
