<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { open as pickPath } from '@tauri-apps/plugin-dialog'
import { settings } from '../../lib/settings'
import { indexStatus as s, indexPause, indexCancel, indexRebuild, indexDelete } from '../../lib/store'
import { fmtSize } from '../../lib/util'
import { t } from '../../lib/i18n'
import { isMac } from '../../lib/util/platform'
import Icon from '../Icon.vue'
import Row from './Row.vue'
import Toggle from './Toggle.vue'

const n = (v: number) => v.toLocaleString()
const stateText = computed(() => {
  if (!settings.fileIndex || s.state === 'off') return t('index.state.off')
  if (s.state === 'cancelled') return t('index.state.cancelled')
  if (s.paused) return t('index.state.paused', { seen: n(s.seen) })
  if (s.state === 'building') return t('index.state.building', { seen: n(s.seen) })
  return s.refreshing ? t('index.state.refreshing', { seen: n(s.seen) }) : t('index.state.ready', { entries: n(s.entries) })
})
const walking = computed(() => s.state === 'building' || s.refreshing)

const excludes = ref(settings.fileIndexExcludes.join('\n'))
watch(excludes, (v) => (settings.fileIndexExcludes = v.split('\n').map((x) => x.trim()).filter(Boolean)))

async function addRoot() {
  const dir = await pickPath({ directory: true, multiple: false })
  if (typeof dir === 'string' && !settings.fileIndexRoots.includes(dir)) settings.fileIndexRoots = [...settings.fileIndexRoots, dir]
}
const removeRoot = (r: string) => (settings.fileIndexRoots = settings.fileIndexRoots.filter((x) => x !== r))
const button = 'h-7 rounded-md bg-raised px-3 text-[12px] text-fg hover:bg-raised/70'
</script>

<template>
  <section class="flex flex-col gap-1">
    <Row :title="t('index.enable')" :desc="t('index.enableDesc')">
      <Toggle v-model="settings.fileIndex" :label="t('index.enable')" />
    </Row>
    <p class="py-1 text-[11.5px] text-dim">{{ t('index.privacy') }}</p>
    <p v-if="isMac && !settings.fileIndex" class="pb-1 text-[11.5px] text-dim">{{ t('index.macPrompts') }}</p>
    <template v-if="settings.fileIndex">
      <Row :title="t('index.status')" :desc="stateText">
        <div class="flex gap-1.5">
          <template v-if="walking">
            <button :class="button" @click="indexPause(!s.paused)">{{ s.paused ? t('index.resume') : t('index.pause') }}</button>
            <button :class="button" @click="indexCancel()">{{ t('index.cancel') }}</button>
          </template>
          <button v-else :class="button" @click="indexRebuild()">{{ s.state === 'ready' ? t('index.rebuild') : t('index.build') }}</button>
        </div>
      </Row>
      <p v-if="s.state === 'ready'" class="text-[11.5px] text-faint">{{ t('index.size', { disk: fmtSize(s.diskBytes), memory: fmtSize(s.memoryBytes) }) }}</p>
      <p v-if="s.watch === 'periodic'" class="text-[11.5px] text-faint">{{ t('index.periodic') }}</p>
      <p v-if="s.truncated" class="text-[11.5px] text-danger">{{ t('index.truncated') }}</p>
      <Row :title="t('index.roots')" :desc="t('index.rootsDesc')">
        <button :class="button" @click="addRoot">{{ t('index.addRoot') }}</button>
      </Row>
      <ul class="flex flex-col gap-0.5 text-[12px] text-dim">
        <li v-if="!settings.fileIndexRoots.length">{{ t('index.rootsDefault') }}</li>
        <li v-for="r in settings.fileIndexRoots" :key="r" class="flex items-center gap-2">
          <span class="truncate">{{ r }}</span>
          <button class="ml-auto text-faint hover:text-fg" :aria-label="t('index.remove', { name: r })" @click="removeRoot(r)"><Icon name="x" class="size-3.5" /></button>
        </li>
      </ul>
      <p v-if="s.skipped.length" class="text-[11.5px] text-danger">{{ t('index.skipped', { paths: s.skipped.join(', ') }) }}</p>
      <div class="py-2.5">
        <div class="text-[12.5px] text-fg">{{ t('index.excludes') }}</div>
        <div class="mt-0.5 text-[11.5px] text-dim">{{ t('index.excludesDesc') }}</div>
        <textarea v-model="excludes" rows="3" spellcheck="false" :aria-label="t('index.excludes')" class="mt-2 w-full rounded-md bg-raised px-2 py-1.5 text-[12px] outline-none focus:ring-1 focus:ring-accent/50" />
      </div>
      <Row :title="t('index.deleteTitle')" :desc="t('index.deleteDesc')">
        <button class="h-7 rounded-md bg-raised px-3 text-[12px] text-danger hover:bg-danger/10" @click="indexDelete">{{ t('index.delete') }}</button>
      </Row>
    </template>
  </section>
</template>
