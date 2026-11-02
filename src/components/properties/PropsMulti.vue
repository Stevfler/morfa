<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../Icon.vue'
import PropRow from './PropRow.vue'
import * as S from '../../lib/store'
import { fmtSize, kindOf, KIND_ICON, KIND_COLOR, KIND_LABEL, type Kind } from '../../lib/util'
import { bytes } from './types'

const props = defineProps<{ paths: string[] }>()
type Stat = { path: string; name: string; isDir: boolean; ext: string; size: number }
const items = ref<Stat[]>([])

watch(
  () => props.paths,
  async (paths) => {
    items.value = []
    const all = await Promise.all(paths.map((path) => invoke<Stat>('stat', { path }).catch(() => null)))
    items.value = all.filter((x): x is Stat => !!x)
    const dirs = items.value.filter((e) => e.isDir).map((e) => e.path)
    if (dirs.length) S.computeSizes(dirs)
  },
  { immediate: true },
)

const pending = computed(() => items.value.some((e) => e.isDir && S.state.sizes[e.path] === 'pending'))
const total = computed(() =>
  items.value.reduce((s, e) => s + (e.isDir ? (typeof S.state.sizes[e.path] === 'number' ? (S.state.sizes[e.path] as number) : 0) : e.size), 0),
)
const kinds = computed(() => {
  const m = new Map<Kind, number>()
  for (const e of items.value) {
    const k = kindOf(e)
    m.set(k, (m.get(k) ?? 0) + 1)
  }
  return [...m].sort((a, b) => b[1] - a[1])
})
const where = computed(() => {
  const dirs = new Set(props.paths.map((p) => p.replace(/[\\/][^\\/]*$/, '') || '/'))
  return dirs.size === 1 ? [...dirs][0] : `${dirs.size} folders`
})
</script>

<template>
  <div class="flex items-center gap-4 px-5 pt-5 pb-4">
    <div class="grid size-14 shrink-0 place-items-center rounded-xl bg-raised text-accent"><Icon name="copy" class="size-8" /></div>
    <div class="min-w-0">
      <p class="text-[15px] font-medium">{{ paths.length }} items</p>
      <p class="truncate text-[12px] text-faint">in {{ where }}</p>
    </div>
  </div>

  <div class="mx-5 mb-3 rounded-lg bg-raised/60 py-1">
    <PropRow label="Total size">
      {{ fmtSize(total) }} <span class="text-faint">({{ bytes(total) }})</span>
      <Icon v-if="pending" name="loader" class="ml-1 inline size-3.5 animate-spin text-faint" />
    </PropRow>
    <PropRow label="Location" :copy="paths.join('\n')">{{ where }}</PropRow>
  </div>

  <div class="mx-5 mb-5 rounded-lg bg-raised/60 py-1.5">
    <div v-for="[k, n] in kinds" :key="k" class="flex items-center gap-2.5 px-3 py-1 text-[12.5px]">
      <Icon :name="KIND_ICON[k]" class="size-4" :style="{ color: KIND_COLOR[k] }" />
      <span>{{ KIND_LABEL[k] }}</span>
      <span class="ml-auto text-faint tabular-nums">{{ n }}</span>
    </div>
  </div>
</template>
