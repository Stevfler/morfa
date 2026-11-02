<script setup lang="ts">
import { computed } from 'vue'
import { DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import type { Entry } from '../../lib/store'
import { fmtDateFull, fmtSize, kindOf, KIND_COLOR, KIND_ICON, KIND_LABEL } from '../../lib/util'

const props = defineProps<{ entry: Entry; pos: string }>()
defineEmits<{ open: [] }>()

const kind = computed(() => kindOf(props.entry))
const meta = computed(() => {
  const e = props.entry
  return [KIND_LABEL[kind.value], e.isDir ? null : fmtSize(e.size), e.modified ? fmtDateFull(e.modified) : null].filter(Boolean).join(' · ')
})
</script>

<template>
  <div class="flex h-14 shrink-0 items-center gap-3 border-b border-line px-4">
    <Icon :name="KIND_ICON[kind]" class="size-6 shrink-0" :style="{ color: KIND_COLOR[kind] }" :stroke-width="1.5" :tint="entry.isDir ? 0.15 : 0" />
    <div class="min-w-0 flex-1">
      <DialogTitle class="truncate text-[14px] font-medium">{{ entry.name }}</DialogTitle>
      <DialogDescription class="truncate text-[11.5px] text-faint">{{ meta }}</DialogDescription>
    </div>
    <span class="shrink-0 text-[11.5px] text-faint tabular-nums">{{ pos }}</span>
    <button
      class="flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-line bg-bg px-3 text-[12.5px] text-dim transition hover:border-accent/50 hover:text-fg"
      @click="$emit('open')"
    >
      <Icon name="external-link" class="size-3.5" /> Open <kbd>Enter</kbd>
    </button>
    <DialogClose class="grid size-8 shrink-0 place-items-center rounded-md text-dim transition hover:bg-raised hover:text-fg" aria-label="Close">
      <Icon name="x" class="size-4" />
    </DialogClose>
  </div>
</template>
