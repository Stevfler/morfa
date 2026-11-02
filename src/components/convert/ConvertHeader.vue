<script setup lang="ts">
import { computed } from 'vue'
import { DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import ConvertThumb from './ConvertThumb.vue'
import { isAudio, isImage, isVideo } from '../../lib/convert'
import { basename } from '../../lib/util'
import { t } from '../../lib/i18n'

const props = defineProps<{ paths: string[] }>()
const title = computed(() => {
  const n = props.paths.length
  if (n === 1) return t('convert.titleOne', { name: basename(props.paths[0]) })
  if (props.paths.every(isImage)) return t('convert.titleMany', { count: n })
  if (props.paths.every(isVideo)) return t('convert.titleManyVideos', { count: n })
  if (props.paths.every(isAudio)) return t('convert.titleManyAudios', { count: n })
  return t('convert.titleManyFiles', { count: n })
})
const thumbs = computed(() => props.paths.slice(0, 6))
</script>

<template>
  <header class="flex items-start gap-4 px-7 pt-6 pb-5">
    <div class="flex shrink-0 -space-x-3">
      <ConvertThumb v-for="(p, i) in thumbs.slice(0, 3)" :key="p" :path="p" :style="{ zIndex: 3 - i }" />
    </div>
    <div class="min-w-0 flex-1">
      <DialogTitle class="truncate text-[16px] font-medium">{{ title }}</DialogTitle>
      <DialogDescription class="mt-1 text-[12px] text-faint">
        {{ t('convert.desc') }}{{ thumbs.length < paths.length ? ` · ${t('convert.moreHidden', { n: paths.length - thumbs.length })}` : '' }}.
      </DialogDescription>
    </div>
    <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" :aria-label="t('common.close')">
      <Icon name="x" class="size-4" />
    </DialogClose>
  </header>
</template>
