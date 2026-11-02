<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from './Icon.vue'
import * as S from '../lib/store'
import { basename, join, keyLabel, parent } from '../lib/util'
import { t } from '../lib/i18n'
import RenameStatus from './batch/RenameStatus.vue'

const state = S.state
const open = computed({ get: () => !!state.batchRename, set: (v) => !v && (state.batchRename = null) })

const find = ref('')
const replace = ref('')
const regex = ref(false)
const matchCase = ref(false)
const template = ref('')
const startAt = ref(1)
const caseMode = ref<'keep' | 'lower' | 'upper' | 'title'>('keep')

watch(open, (v) => {
  if (v) {
    find.value = replace.value = template.value = ''
    startAt.value = 1
    caseMode.value = 'keep'
  }
})

const preview = computed(() => {
  const paths = state.batchRename ?? []
  const pad = String(paths.length + startAt.value - 1).length
  let err = ''
  const rows = paths.map((path, i) => {
    const name = basename(path)
    const dot = name.lastIndexOf('.')
    const stem = dot > 0 ? name.slice(0, dot) : name
    const ext = dot > 0 ? name.slice(dot) : ''
    let next = name
    if (template.value) {
      // {name} = original stem, {n} = counter, {ext} keeps the extension automatically
      next = template.value.replace(/\{name\}/g, stem).replace(/\{n\}/g, String(i + startAt.value).padStart(pad, '0')) + ext
    }
    if (find.value) {
      try {
        const re = regex.value
          ? new RegExp(find.value, matchCase.value ? 'g' : 'gi')
          : new RegExp(find.value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), matchCase.value ? 'g' : 'gi')
        next = next.replace(re, replace.value)
      } catch (e) {
        err = String(e)
      }
    }
    if (caseMode.value !== 'keep') {
      const d = next.lastIndexOf('.')
      const [s, x] = d > 0 ? [next.slice(0, d), next.slice(d)] : [next, '']
      const conv = caseMode.value === 'lower' ? s.toLowerCase() : caseMode.value === 'upper' ? s.toUpperCase() : s.toLowerCase().replace(/(^|[\s_-])\p{L}/gu, (m) => m.toUpperCase())
      next = conv + x
    }
    return { path, name, next, changed: next !== name }
  })
  const seen = new Map<string, number>()
  rows.forEach((r) => seen.set(r.next.toLowerCase(), (seen.get(r.next.toLowerCase()) ?? 0) + 1))
  const dupes = rows.some((r) => (seen.get(r.next.toLowerCase()) ?? 0) > 1)
  const invalid = rows.some((r) => !r.next.trim() || /[\\/]/.test(r.next))
  return { rows, err, dupes, invalid, count: rows.filter((r) => r.changed).length }
})

function apply() {
  const pv = preview.value
  if (pv.dupes || pv.invalid || pv.err || !pv.count) return
  const pairs = pv.rows.filter((r) => r.changed).map((r) => [r.path, join(parent(r.path)!, r.next)] as [string, string])
  // Two-phase via temp names so swaps like a→b, b→a don't collide.
  const tmp = pairs.map(([from]) => [from, from + `.morfa-tmp-${Date.now()}`] as [string, string])
  const steps: [string, string][] = [...tmp, ...tmp.map(([, t], i) => [t, pairs[i][1]] as [string, string])]
  state.batchRename = null
  S.movePairs(steps, t('batch.done', { count: pairs.length }))
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 flex max-h-[86vh] w-[min(760px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none">
        <div class="px-5 pt-5 pb-3">
          <DialogTitle class="text-[15px] font-medium">{{ t('batch.title', { count: state.batchRename?.length ?? 0 }) }}</DialogTitle>
          <DialogDescription class="mt-0.5 text-[12px] text-faint">{{ t('batch.desc', { key: keyLabel('Ctrl+Z') }) }}</DialogDescription>
        </div>

        <div class="grid grid-cols-2 gap-4 px-5 py-4">
          <label class="field">
            <span>{{ t('batch.find') }}</span>
            <input v-model="find" :placeholder="t('batch.findPh')" spellcheck="false" autofocus />
          </label>
          <label class="field">
            <span>{{ t('batch.replace') }}</span>
            <input v-model="replace" :placeholder="t('batch.replacePh')" spellcheck="false" />
          </label>
          <label class="field">
            <span>{{ t('batch.pattern') }} <em class="text-faint not-italic">{{ t('batch.optional') }}</em></span>
            <input v-model="template" :placeholder="t('batch.patternPh')" spellcheck="false" />
          </label>
          <div class="flex items-end gap-3">
            <label class="field w-20"><span>{{ t('batch.from') }}</span><input v-model.number="startAt" type="number" min="0" /></label>
            <label class="field flex-1">
              <span>{{ t('batch.case') }}</span>
              <select v-model="caseMode">
                <option value="keep">{{ t('batch.keep') }}</option><option value="lower">{{ t('batch.lower') }}</option>
                <option value="upper">{{ t('batch.upper') }}</option><option value="title">{{ t('batch.titleCase') }}</option>
              </select>
            </label>
          </div>
          <div class="col-span-2 flex gap-4 text-[12px] text-dim">
            <label class="flex items-center gap-1.5"><input v-model="regex" type="checkbox" class="accent-accent" />{{ t('batch.regex') }}</label>
            <label class="flex items-center gap-1.5"><input v-model="matchCase" type="checkbox" class="accent-accent" />{{ t('batch.matchCase') }}</label>
          </div>
        </div>

        <div class="mx-5 min-h-0 flex-1 overflow-y-auto rounded-lg bg-raised/60">
          <div v-for="r in preview.rows" :key="r.path" class="grid grid-cols-[1fr_auto_1fr] items-center gap-3 rounded-md px-3 py-1.5 text-[12px] odd:bg-raised">
            <span class="truncate text-dim">{{ r.name }}</span>
            <Icon name="arrow-right" class="size-3.5" :class="r.changed ? 'text-accent' : 'text-line'" />
            <span class="truncate" :class="r.changed ? 'text-fg' : 'text-faint'">{{ r.next }}</span>
          </div>
        </div>

        <div class="flex items-center gap-3 px-5 py-4">
          <RenameStatus :err="preview.err" :dupes="preview.dupes" :invalid="preview.invalid" :count="preview.count" :total="preview.rows.length" />
          <DialogClose class="ml-auto h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">{{ t('common.cancel') }}</DialogClose>
          <button
            class="h-8 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent transition hover:brightness-110 disabled:opacity-40"
            :disabled="!!preview.err || preview.dupes || preview.invalid || !preview.count" @click="apply"
          >{{ t('batch.apply', { count: preview.count || '' }) }}</button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
@reference '../style.css';
.field {
  @apply flex flex-col gap-1 text-[11.5px] text-dim;
}
.field input:not([type='checkbox']),
.field select {
  @apply h-8 rounded-full bg-raised px-3 text-[12.5px] text-fg outline-none focus:ring-1 focus:ring-accent/50;
}
</style>
