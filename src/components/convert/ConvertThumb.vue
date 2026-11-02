<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../Icon.vue'
import { isImage, extOf } from '../../lib/convert'
import { basename, kindOf, KIND_COLOR, KIND_ICON } from '../../lib/util'
import { useImageUrl } from '../../composables/useImageUrl'

const props = defineProps<{ path: string }>()
const image = computed(() => isImage(props.path))
const url = useImageUrl(() => (image.value ? props.path : null))
const kind = computed(() => kindOf({ isDir: false, ext: extOf(props.path), name: basename(props.path) }))
</script>

<template>
  <img v-if="image && url" :src="url" alt="" class="size-12 rounded-lg bg-raised object-cover ring-2 ring-panel" />
  <div v-else class="grid size-12 place-items-center rounded-lg bg-raised ring-2 ring-panel">
    <Icon v-if="!image" :name="KIND_ICON[kind]" class="size-6" :style="{ color: KIND_COLOR[kind] }" />
  </div>
</template>
