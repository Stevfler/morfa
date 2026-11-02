<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../Icon.vue'
import PropRow from './PropRow.vue'
import { basename, fmtSize, kindOf, KIND_ICON, KIND_COLOR, KIND_LABEL } from '../../lib/util'
import { t } from '../../lib/i18n'
import { bytes, fullDate, octal, rwx, type Props } from './types'

const props = defineProps<{ path: string }>()
const info = ref<Props | null>(null)
const error = ref('')
const walk = ref<null | 'pending' | { bytes: number; files: number }>(null)

watch(
  () => props.path,
  async (path) => {
    info.value = null
    error.value = ''
    walk.value = null
    try {
      info.value = await invoke<Props>('properties', { path })
    } catch (e) {
      error.value = String(e)
      return
    }
    if (!info.value.isDir) return
    walk.value = 'pending'
    invoke<[number, number]>('dir_size', { path })
      .then(([b, f]) => props.path === path && (walk.value = { bytes: b, files: f }))
      .catch(() => props.path === path && (walk.value = null))
  },
  { immediate: true },
)

const name = computed(() => basename(props.path) || props.path)
const ext = computed(() => (info.value?.isDir ? '' : (name.value.match(/\.([^.]+)$/)?.[1] ?? '').toLowerCase()))
const kind = computed(() => kindOf({ isDir: !!info.value?.isDir, ext: ext.value, name: name.value }))
</script>

<template>
  <p v-if="error" class="flex items-center gap-1.5 px-5 py-6 text-[12.5px] text-danger"><Icon name="warning" class="size-4" />{{ error }}</p>
  <div v-else-if="!info" class="flex justify-center py-10"><Icon name="loader" class="size-5 animate-spin text-faint" /></div>
  <template v-else>
    <div class="flex items-center gap-4 px-5 pt-5 pb-4">
      <div class="grid size-14 shrink-0 place-items-center rounded-xl bg-raised" :style="{ color: KIND_COLOR[kind] }">
        <Icon :name="KIND_ICON[kind]" class="size-8" />
      </div>
      <div class="min-w-0">
        <p class="truncate text-[15px] font-medium select-text">{{ name }}</p>
        <p class="text-[12px] text-faint">{{ info.isSymlink ? t('props.linkTo', { kind: KIND_LABEL[kind] }) : KIND_LABEL[kind] }}{{ ext ? ` · .${ext}` : '' }}</p>
      </div>
    </div>

    <div class="mx-5 mb-3 rounded-lg bg-raised/60 py-1">
      <PropRow :label="t('props.location')" :copy="info.path">{{ info.path }}</PropRow>
      <PropRow v-if="info.linkTarget" :label="t('props.pointsTo')" :copy="info.linkTarget">{{ info.linkTarget }}</PropRow>
      <PropRow :label="t('props.size')">
        <template v-if="!info.isDir">{{ fmtSize(info.size) }} <span class="text-faint">({{ bytes(info.size) }})</span></template>
        <span v-else-if="walk === 'pending'" class="inline-flex items-center gap-1.5 text-faint"><Icon name="loader" class="size-3.5 animate-spin" />{{ t('props.calculating') }}</span>
        <template v-else-if="walk">{{ fmtSize(walk.bytes) }} <span class="text-faint">({{ bytes(walk.bytes) }})</span></template>
        <span v-else class="text-faint">{{ t('props.unknown') }}</span>
      </PropRow>
      <PropRow v-if="info.isDir" :label="t('props.contains')">
        {{ t('props.itemsHere', { count: info.children ?? 0 }) }}<template v-if="walk && walk !== 'pending'"> · {{ t('props.filesTotal', { count: walk.files }) }}</template>
      </PropRow>
      <PropRow v-if="info.width && info.height" :label="t('props.dimensions')">{{ info.width }} × {{ info.height }} {{ t('props.px') }}</PropRow>
    </div>

    <div class="mx-5 mb-3 rounded-lg bg-raised/60 py-1">
      <PropRow :label="t('props.created')">{{ fullDate(info.created) }}</PropRow>
      <PropRow :label="t('props.modified')">{{ fullDate(info.modified) }}</PropRow>
      <PropRow :label="t('props.accessed')">{{ fullDate(info.accessed) }}</PropRow>
    </div>

    <div class="mx-5 mb-5 rounded-lg bg-raised/60 py-1">
      <PropRow v-if="info.mode != null" :label="t('props.permissions')" mono>{{ rwx(info.mode) }} <span class="text-faint">({{ octal(info.mode) }})</span></PropRow>
      <PropRow v-else :label="t('props.access')">{{ info.readonly ? t('props.readonly') : t('props.readwrite') }}</PropRow>
      <PropRow v-if="info.owner" :label="t('props.owner')">{{ info.owner }}<span class="text-faint"> : {{ info.group }}</span></PropRow>
      <PropRow v-if="info.mount" :label="t('props.drive')" :copy="info.mount">
        {{ info.mount }} <span class="text-faint">· {{ info.fsType }} · {{ info.device }}</span>
      </PropRow>
    </div>
  </template>
</template>
