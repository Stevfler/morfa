<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../Icon.vue'
import * as S from '../../lib/store'
import type { Entry, Pane } from '../../lib/store'
import { crumbs, keyLabel, parent } from '../../lib/util'
import type { DragDrop } from '../../composables/useDragDrop'

const props = defineProps<{ pane: Pane; active: boolean; dnd: DragDrop }>()
const p = props.pane
const state = S.state
const { dropTarget, dragOver, dragLeave, drop } = props.dnd

const pathInput = ref<HTMLInputElement>()
const pathText = ref('')
const completions = ref<string[]>([])
const editing = computed(() => state.editPath && props.active)
watch(editing, async (v) => {
  if (!v) return
  pathText.value = p.path
  await nextTick()
  pathInput.value?.select()
})
watch(pathText, async (v) => {
  if (!editing.value) return
  completions.value = v ? await invoke<string[]>('complete_path', { input: v }).catch(() => []) : []
})
async function commitPath(target = pathText.value) {
  const t = target.trim().replace(/^~(?=$|[\\/])/, state.home)
  try {
    const e = await invoke<Entry>('stat', { path: t })
    state.editPath = false
    e.isDir ? S.go(p, e.path) : S.go(p, parent(e.path)!, { focus: e.path })
  } catch {
    S.toast({ title: 'Folder not found', desc: t, tone: 'error' })
  }
}
function pathKey(ev: KeyboardEvent) {
  if (ev.key === 'Enter') commitPath()
  else if (ev.key === 'Escape') state.editPath = false
  else if (ev.key === 'Tab' && completions.value[0]) {
    ev.preventDefault()
    pathText.value = completions.value[0] + (completions.value[0].includes('\\') ? '\\' : '/')
  } else return
  ev.stopPropagation()
}
</script>

<template>
  <!-- path: breadcrumbs that turn into a text field -->
  <div class="relative mx-1 min-w-0 flex-1">
    <template v-if="editing">
      <input
        ref="pathInput" v-model="pathText"
        class="h-7 w-full rounded-md border border-accent/60 bg-panel px-2 font-mono text-[12px] outline-none"
        spellcheck="false" @keydown="pathKey" @blur="state.editPath = false"
      />
      <div v-if="completions.length" class="absolute inset-x-0 top-8 z-30 overflow-hidden rounded-md border border-line bg-raised shadow-2xl">
        <button
          v-for="(c, k) in completions" :key="c"
          class="flex w-full items-center gap-2 px-2 py-1 text-left font-mono text-[12px] hover:bg-accent-soft"
          :class="k === 0 && 'bg-accent-soft/50'" @mousedown.prevent="commitPath(c)"
        >
          <Icon name="folder-open" class="size-3.5 text-accent" />{{ c }}
          <kbd v-if="k === 0" class="ml-auto">Tab</kbd>
        </button>
      </div>
    </template>
    <div
      v-else
      class="flex h-7 cursor-text items-center overflow-hidden rounded-md px-1 hover:bg-panel"
      :title="`Click empty space or press ${keyLabel('Ctrl+L')} to type a path`"
      @click.self="state.editPath = true; S.focusPane(S.tab().panes.indexOf(p))"
    >
      <template v-for="(c, k) in crumbs(p.path)" :key="c.path">
        <Icon name="chevron-right" v-if="k > 0" class="size-3.5 shrink-0 text-faint" />
        <button
          class="shrink-0 truncate rounded px-1.5 py-0.5 text-[12.5px] transition hover:bg-raised"
          :class="[k === crumbs(p.path).length - 1 ? 'font-medium text-fg' : 'text-dim', dropTarget === c.path && 'bg-accent-soft ring-1 ring-accent']"
          @click="S.go(p, c.path)" @dragover="dragOver($event, c.path)" @dragleave="dragLeave" @drop="drop($event, c.path)"
        >{{ c.label }}</button>
      </template>
      <div class="h-full flex-1" @click="state.editPath = true" />
    </div>
  </div>
</template>
