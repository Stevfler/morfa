<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import { ArrowRight, TriangleAlert } from 'lucide-vue-next'
import * as S from '../lib/store'
import { basename, join, keyLabel, parent } from '../lib/util'

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
  const tmp = pairs.map(([from]) => [from, from + `.orbit-tmp-${Date.now()}`] as [string, string])
  const steps: [string, string][] = [...tmp, ...tmp.map(([, t], i) => [t, pairs[i][1]] as [string, string])]
  state.batchRename = null
  S.movePairs(steps, `Renamed ${pairs.length} items`)
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-black/50 backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 flex max-h-[86vh] w-[min(760px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl border border-line bg-panel shadow-2xl outline-none">
        <div class="border-b border-line px-5 py-4">
          <DialogTitle class="text-[15px] font-medium">Rename {{ state.batchRename?.length }} items</DialogTitle>
          <DialogDescription class="mt-0.5 text-[12px] text-faint">Every change is previewed live. Extensions are kept. You can undo with {{ keyLabel('Ctrl+Z') }}.</DialogDescription>
        </div>

        <div class="grid grid-cols-2 gap-4 px-5 py-4">
          <label class="field">
            <span>Find</span>
            <input v-model="find" placeholder="text to replace" spellcheck="false" autofocus />
          </label>
          <label class="field">
            <span>Replace with</span>
            <input v-model="replace" placeholder="(leave empty to remove)" spellcheck="false" />
          </label>
          <label class="field">
            <span>New name pattern <em class="text-faint not-italic">— optional</em></span>
            <input v-model="template" placeholder="e.g. Trip {n} or {name}-final" spellcheck="false" />
          </label>
          <div class="flex items-end gap-3">
            <label class="field w-20"><span>{n} from</span><input v-model.number="startAt" type="number" min="0" /></label>
            <label class="field flex-1">
              <span>Letter case</span>
              <select v-model="caseMode">
                <option value="keep">Keep</option><option value="lower">lowercase</option>
                <option value="upper">UPPERCASE</option><option value="title">Title Case</option>
              </select>
            </label>
          </div>
          <div class="col-span-2 flex gap-4 text-[12px] text-dim">
            <label class="flex items-center gap-1.5"><input v-model="regex" type="checkbox" class="accent-accent" />Regular expression</label>
            <label class="flex items-center gap-1.5"><input v-model="matchCase" type="checkbox" class="accent-accent" />Match case</label>
          </div>
        </div>

        <div class="mx-5 min-h-0 flex-1 overflow-y-auto rounded-lg border border-line bg-bg/60">
          <div v-for="r in preview.rows" :key="r.path" class="grid grid-cols-[1fr_auto_1fr] items-center gap-3 border-b border-line/60 px-3 py-1.5 text-[12px] last:border-0">
            <span class="truncate text-dim">{{ r.name }}</span>
            <ArrowRight class="size-3.5" :class="r.changed ? 'text-accent' : 'text-line'" />
            <span class="truncate" :class="r.changed ? 'text-fg' : 'text-faint'">{{ r.next }}</span>
          </div>
        </div>

        <div class="flex items-center gap-3 px-5 py-4">
          <p v-if="preview.err" class="flex items-center gap-1.5 text-[12px] text-danger"><TriangleAlert class="size-4" />Invalid pattern</p>
          <p v-else-if="preview.dupes" class="flex items-center gap-1.5 text-[12px] text-warn"><TriangleAlert class="size-4" />Some names would be the same — add {n}</p>
          <p v-else-if="preview.invalid" class="flex items-center gap-1.5 text-[12px] text-warn"><TriangleAlert class="size-4" />Names can’t be empty or contain / \</p>
          <p v-else class="text-[12px] text-faint">{{ preview.count }} of {{ preview.rows.length }} will change</p>
          <DialogClose class="ml-auto h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">Cancel</DialogClose>
          <button
            class="h-8 rounded-md bg-accent px-4 text-[12.5px] font-medium text-white transition hover:brightness-110 disabled:opacity-40"
            :disabled="!!preview.err || preview.dupes || preview.invalid || !preview.count" @click="apply"
          >Rename {{ preview.count || '' }}</button>
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
  @apply h-8 rounded-md border border-line bg-bg px-2.5 text-[12.5px] text-fg outline-none focus:border-accent/60;
}
</style>
