<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, triggerRef, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from './Icon.vue'
import * as S from '../lib/store'
import { basename, join, keyLabel, parent } from '../lib/util'
import { t } from '../lib/i18n'
import RenameStatus from './batch/RenameStatus.vue'
import { CHIPS, expand, lowercaseOf, parseTemplate, placesExt, tempName, type FileMeta, type Group } from './batch/tokens'

const state = S.state
const open = computed({ get: () => !!state.batchRename, set: (v) => !v && (state.batchRename = null) })

const find = ref('')
const replace = ref('')
const regex = ref(false)
const matchCase = ref(false)
const template = ref('')
const startAt = ref(1)
const caseMode = ref<'keep' | 'lower' | 'upper' | 'title'>('keep')
const onMissing = ref<'leave' | 'text'>('leave')
const fallback = ref('')
const excluded = ref(new Set<string>())
const patternInput = ref<HTMLInputElement | null>(null)

// ---- Metadata: read in small batches behind the preview, so typing never waits for it.
const CHUNK = 32
const metas = shallowRef(new Map<string, FileMeta>())
const groupState = ref<Record<Group, 'idle' | 'loading' | 'done'>>({ stat: 'idle', image: 'idle', audio: 'idle' })
const readFailed = ref(false)
let session = 0
let nextCall = Date.now()
const running = new Set<number>()

function cancelRunning() {
  for (const id of running) invoke('rename_cancel', { id }).catch(() => {})
  running.clear()
}

async function loadGroup(g: Group, paths: string[], own: number) {
  groupState.value[g] = 'loading'
  for (let i = 0; i < paths.length; i += CHUNK) {
    if (own !== session) return
    const id = ++nextCall
    running.add(id)
    try {
      const got = await invoke<FileMeta[]>('rename_meta_read', { id, paths: paths.slice(i, i + CHUNK), image: g === 'image', audio: g === 'audio' })
      if (own !== session) return
      for (const m of got) {
        const had = metas.value.get(m.path)
        if (!had) metas.value.set(m.path, m)
        // Each read only fills its own fields.
        else for (const [k, v] of Object.entries(m)) if (v !== null) (had as unknown as Record<string, unknown>)[k] = v
      }
      triggerRef(metas)
    } catch {
      // These files simply have no values: they're flagged as missing, and the line below says why.
      if (own !== session) return
      readFailed.value = true
    } finally {
      running.delete(id)
    }
  }
  groupState.value[g] = 'done'
}

const parsed = computed(() => parseTemplate(template.value))
watch(
  () => [...parsed.value.groups].join(),
  () => {
    for (const g of parsed.value.groups) if (groupState.value[g] === 'idle') void loadGroup(g, state.batchRename ?? [], session)
  },
)
const loading = computed(() => [...parsed.value.groups].some((g) => groupState.value[g] !== 'done'))

watch(open, (v) => {
  session++
  cancelRunning()
  cancelCheck()
  if (v) {
    find.value = replace.value = template.value = fallback.value = ''
    startAt.value = 1
    caseMode.value = 'keep'
    onMissing.value = 'leave'
    excluded.value = new Set()
    metas.value = new Map()
    groupState.value = { stat: 'idle', image: 'idle', audio: 'idle' }
    readFailed.value = false
    checks.value = new Map()
    checkFailed.value = false
  }
})
onBeforeUnmount(() => {
  session++
  cancelRunning()
  cancelCheck()
})

const chipLabel = (name: string) => '{' + name + '}'

function insertToken(name: string) {
  const el = patternInput.value
  const token = `{${name}}`
  const at = el?.selectionStart ?? template.value.length
  const end = el?.selectionEnd ?? at
  template.value = template.value.slice(0, at) + token + template.value.slice(end)
  requestAnimationFrame(() => {
    el?.focus()
    el?.setSelectionRange(at + token.length, at + token.length)
  })
}

// ---- The preview: computed here at once; what the disk says arrives separately.
const preview = computed(() => {
  const paths = state.batchRename ?? []
  const pad = String(paths.length + startAt.value - 1).length
  const tpl = template.value
  const text = onMissing.value === 'text' ? fallback.value.trim() : ''
  let err = ''
  let re: RegExp | null = null
  if (find.value) {
    try {
      re = regex.value
        ? new RegExp(find.value, matchCase.value ? 'g' : 'gi')
        : new RegExp(find.value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), matchCase.value ? 'g' : 'gi')
    } catch (e) {
      err = String(e)
    }
  }
  const rows = paths.map((path, i) => {
    const name = basename(path)
    const dot = name.lastIndexOf('.')
    const stem = dot > 0 ? name.slice(0, dot) : name
    const ext = dot > 0 ? name.slice(dot) : ''
    let next = name
    let missing: string[] = []
    if (tpl) {
      const n = String(i + startAt.value).padStart(pad, '0')
      const r = expand(tpl, { stem, ext: ext.slice(1), n, meta: metas.value.get(path) }, text)
      missing = r.missing
      // {ext} placed at the end replaces the automatic one. No extension: no dangling dot.
      next = placesExt(tpl) ? (ext ? r.text : r.text.replace(/\.$/, '')) : r.text + ext
    }
    const left = missing.length > 0 && !text
    if (left) next = name
    else {
      if (re) next = next.replace(re, replace.value)
      if (caseMode.value !== 'keep') {
        const d = next.lastIndexOf('.')
        const [s, x] = d > 0 ? [next.slice(0, d), next.slice(d)] : [next, '']
        const conv = caseMode.value === 'lower' ? s.toLowerCase() : caseMode.value === 'upper' ? s.toUpperCase() : s.toLowerCase().replace(/(^|[\s_-])\p{L}/gu, (m) => m.toUpperCase())
        next = conv + x
      }
    }
    const out = excluded.value.has(path)
    return { path, name, wanted: next, next: out ? name : next, missing, left, out }
  })
  return { rows, err }
})

// ---- The check: judged by the backend, per file system. Only the newest answer counts.
type Check = { status: 'ok' | 'same' | 'duplicate' | 'exists' | 'gone' } | { status: 'invalid'; detail: 'empty' | 'dots' | 'chars' | 'trailing' | 'reserved' | 'long' | 'unreadable' | 'pseudo' }
const checks = shallowRef(new Map<string, { next: string; check: Check }>())
const checkFailed = ref(false)
let checkId = 0
let checkTimer: ReturnType<typeof setTimeout> | undefined

function cancelCheck() {
  clearTimeout(checkTimer)
  if (checkId) invoke('rename_cancel', { id: checkId }).catch(() => {})
  checkId = 0
}

/** Rows stay "checking" until an answer for exactly their current name is in. */
async function runCheck(): Promise<boolean> {
  cancelCheck()
  const rows = preview.value.rows
  if (!rows.length) return true
  const id = (checkId = ++nextCall)
  try {
    const got = await invoke<Check[]>('rename_check', { id, rows: rows.map((r) => ({ path: r.path, next: r.next })) })
    if (id !== checkId) return false
    checks.value = new Map(rows.map((r, i) => [r.path, { next: r.next, check: got[i] }]))
    checkFailed.value = false
    return true
  } catch {
    if (id === checkId) checkFailed.value = true
    return false
  }
}

const sentKey = computed(() => preview.value.rows.map((r) => r.next).join('\0'))
watch(sentKey, () => {
  clearTimeout(checkTimer)
  checkTimer = setTimeout(() => void runCheck(), 120)
}, { immediate: true })

const rowsView = computed(() =>
  preview.value.rows.map((r) => {
    const c = checks.value.get(r.path)
    const check: Check | null = c && c.next === r.next ? c.check : null
    const conflict = !!check && !['ok', 'same'].includes(check.status)
    return { ...r, check, conflict, changed: r.next !== r.name, hidden: r.next !== r.name && r.next.startsWith('.') && !r.name.startsWith('.') }
  }),
)
const counts = computed(() => {
  const v = rowsView.value
  return {
    conflicts: v.filter((r) => r.conflict).length,
    missing: v.filter((r) => r.left && !r.out).length,
    excluded: v.filter((r) => r.out).length,
    changed: v.filter((r) => r.changed).length,
    pending: v.some((r) => !r.check),
  }
})
const anyMissing = computed(() => !loading.value && preview.value.rows.some((r) => r.missing.length))
const unknownToken = computed(() => parsed.value.unknown[0] ?? '')
const unknownHint = computed(() => lowercaseOf(unknownToken.value) ?? '')
const blocked = computed(() => !!preview.value.err || !!unknownToken.value || loading.value || checkFailed.value || counts.value.pending || counts.value.conflicts > 0 || !counts.value.changed)

function setExcluded(path: string, out: boolean) {
  const s = new Set(excluded.value)
  if (out) s.add(path)
  else s.delete(path)
  excluded.value = s
}
function excludeConflicts() {
  const s = new Set(excluded.value)
  for (const r of rowsView.value) if (r.conflict) s.add(r.path)
  excluded.value = s
}

function rowNote(r: (typeof rowsView.value)[number]): string {
  if (r.out) return t('batch.row.excluded')
  if (r.left && loading.value) return t('batch.row.reading')
  if (r.left) return t('batch.row.missing', { tokens: r.missing.join(', ') })
  const c = r.check
  if (!c) return ''
  if (c.status === 'duplicate') return t('batch.row.duplicate')
  if (c.status === 'exists') return t('batch.row.exists')
  if (c.status === 'gone') return t('batch.row.gone')
  if (c.status === 'invalid') return t(`batch.row.invalid.${c.detail}`)
  if (r.missing.length) return t('batch.row.fallback', { tokens: r.missing.join(', ') })
  return r.hidden ? t('batch.row.hidden') : ''
}

async function apply() {
  if (blocked.value) return
  // The disk may have changed since the last answer.
  if (!(await runCheck()) || blocked.value) return
  const pairs = rowsView.value.filter((r) => r.changed).map((r) => [r.path, join(parent(r.path)!, r.next)] as [string, string])
  // Two-phase via temp names so swaps like a→b, b→a don't collide. A file left there by a failure still shows what it is.
  const stamp = Date.now().toString(36)
  const tmp = pairs.map(([from], i) => [from, join(parent(from)!, tempName(basename(pairs[i][1]), stamp, i))] as [string, string])
  const steps: [string, string][] = [...tmp, ...tmp.map(([, t], i) => [t, pairs[i][1]] as [string, string])]
  state.batchRename = null
  S.movePairs(steps, t('batch.done', { count: pairs.length }), 'batchRename', pairs)
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 flex max-h-[86vh] w-[min(820px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none">
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
            <input ref="patternInput" v-model="template" :placeholder="t('batch.patternPh')" spellcheck="false" />
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
          <div class="col-span-2 flex flex-wrap gap-1">
            <button v-for="c in CHIPS" :key="c" type="button" class="rounded-full bg-raised px-2 py-0.5 font-mono text-[11px] text-dim hover:text-fg" :title="t(`batch.token.${c}`)" @click="insertToken(c)">{{ chipLabel(c) }}</button>
          </div>
          <p class="col-span-2 text-[11px] text-faint">{{ t('batch.tokenHelp') }}</p>
          <div class="col-span-2 flex gap-4 text-[12px] text-dim">
            <label class="flex items-center gap-1.5"><input v-model="regex" type="checkbox" class="accent-accent" />{{ t('batch.regex') }}</label>
            <label class="flex items-center gap-1.5"><input v-model="matchCase" type="checkbox" class="accent-accent" />{{ t('batch.matchCase') }}</label>
          </div>
          <div v-if="anyMissing" class="col-span-2 flex flex-wrap items-center gap-3 text-[12px] text-dim">
            <span class="text-warn">{{ t('batch.missingTitle') }}</span>
            <label class="flex items-center gap-1.5"><input v-model="onMissing" type="radio" value="leave" class="accent-accent" />{{ t('batch.missingLeave') }}</label>
            <label class="flex items-center gap-1.5"><input v-model="onMissing" type="radio" value="text" class="accent-accent" />{{ t('batch.missingText') }}</label>
            <input v-if="onMissing === 'text'" v-model="fallback" class="h-7 rounded-full bg-raised px-3 text-[12px] text-fg outline-none focus:ring-1 focus:ring-accent/50" :placeholder="t('batch.missingPh')" spellcheck="false" />
          </div>
        </div>

        <div class="mx-5 min-h-24 flex-1 overflow-y-auto rounded-lg bg-raised/60">
          <div v-for="r in rowsView" :key="r.path" class="grid grid-cols-[auto_1fr_auto_1fr] items-center gap-3 rounded-md px-3 py-1.5 text-[12px] odd:bg-raised" :class="r.out && 'opacity-50'">
            <input type="checkbox" class="accent-accent" :checked="!r.out" :aria-label="t('batch.row.include', { name: r.name })" @change="setExcluded(r.path, !($event.target as HTMLInputElement).checked)" />
            <span class="truncate text-dim" :title="r.name">{{ r.name }}</span>
            <Icon name="arrow-right" class="size-3.5" :class="r.conflict ? 'text-warn' : r.changed ? 'text-accent' : 'text-line'" />
            <span class="min-w-0">
              <span class="block truncate" :class="r.conflict ? 'text-warn' : r.changed ? 'text-fg' : 'text-faint'" :title="r.conflict ? r.wanted : r.next">{{ r.left ? t('batch.row.unchanged') : r.wanted }}</span>
              <span v-if="rowNote(r)" class="block truncate text-[11px]" :class="r.conflict || (r.left && !loading) || r.hidden ? 'text-warn' : 'text-faint'" :title="rowNote(r)">{{ rowNote(r) }}</span>
            </span>
          </div>
        </div>

        <div class="flex items-center gap-3 px-5 py-4">
          <RenameStatus :err="preview.err" :unknown="unknownToken" :unknown-hint="unknownHint" :failed="checkFailed" :read-failed="readFailed" :busy="loading || counts.pending" :conflicts="counts.conflicts" :missing="counts.missing" :excluded="counts.excluded" :count="counts.changed" :total="rowsView.length" />
          <button v-if="counts.conflicts" class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised" @click="excludeConflicts">{{ t('batch.excludeConflicts', { count: counts.conflicts }) }}</button>
          <DialogClose class="ml-auto h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">{{ t('common.cancel') }}</DialogClose>
          <button
            class="h-8 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent transition hover:brightness-110 disabled:opacity-40"
            :disabled="blocked" @click="apply"
          >{{ t('batch.apply', { count: counts.changed }) }}</button>
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
