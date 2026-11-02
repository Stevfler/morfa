<script setup lang="ts">
import { computed } from 'vue'
import { colorOf, tagLabel, tagsOf } from '../../lib/tags'

const props = defineProps<{ path: string }>()
const tags = computed(() => tagsOf(props.path))
const colors = computed(() => tags.value.map(colorOf).filter(Boolean) as string[])
</script>

<template>
  <span v-if="tags.length" class="flex shrink-0 items-center" :title="tags.map(tagLabel).join(', ')">
    <span
      v-for="(c, i) in colors" :key="c" class="size-2 rounded-full ring-1 ring-panel"
      :class="i > 0 && '-ml-0.5'" :style="{ background: c }"
    />
    <span v-if="tags.length > colors.length" class="ml-1 text-[10.5px] text-faint">#{{ tags.length - colors.length }}</span>
  </span>
</template>
