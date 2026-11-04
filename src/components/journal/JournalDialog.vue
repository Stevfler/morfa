<script setup lang="ts">
import { computed, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import { invoke } from '@tauri-apps/api/core'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import Icon from '../Icon.vue'
import * as S from '../../lib/store'
import { settings } from '../../lib/settings'
import { fmtDateFull, isMac } from '../../lib/util'
import { locale, t, type MsgKey } from '../../lib/i18n'
import { jr, KINDS, closeJournal, confirmClear, expand, findWhere, load, nameOf, overridable, runUndo, startUndo, type Now, type Row } from './journal'

const open = computed({ get: () => jr.open, set: (v) => !v && closeJournal() })
const tr = (k: string, p?: Record<string, string | number>) => t(k as MsgKey, p)

let timer: ReturnType<typeof setTimeout> | undefined
watch(() => [jr.search, jr.kind], () => {
  clearTimeout(timer)
  timer = setTimeout(() => load(), 150)
})
let whereTimer: ReturnType<typeof setTimeout> | undefined
watch(() => jr.whereQuery, () => {
  clearTimeout(whereTimer)
  whereTimer = setTimeout(findWhere, 200)
})

const dayStart = (ms: number) => new Date(ms).setHours(0, 0, 0, 0)
function dayLabel(ms: number) {
  const today = dayStart(Date.now())
  const d = dayStart(ms)
  if (d === today) return t('journal.today')
  if (d === new Date(today).setDate(new Date(today).getDate() - 1)) return t('journal.yesterday')
  return new Intl.DateTimeFormat(locale.value, { weekday: 'short', day: 'numeric', month: 'long', year: 'numeric' }).format(ms)
}
const days = computed(() => {
  const out: { key: number; label: string; rows: Row[] }[] = []
  for (const r of jr.rows) {
    const key = dayStart(r.time)
    if (out[out.length - 1]?.key !== key) out.push({ key, label: dayLabel(r.time), rows: [] })
    out[out.length - 1].rows.push(r)
  }
  return out
})
const timeOf = (ms: number) => new Intl.DateTimeFormat(locale.value, { hour: '2-digit', minute: '2-digit' }).format(ms)
const nowTone = (n: Now) => (n === 'here' || n === 'moved' ? 'text-dim' : 'text-danger')
const canReveal = (n: Now) => n === 'here' || n === 'moved'
const reveal = (p: string) => revealItemInDir(p).catch(S.fail)
const showTrash = () => invoke('reveal_trash').catch(S.fail)

const ready = computed(() => jr.preview?.items.filter((i) => !i.reason || (overridable(i.reason) && jr.named.has(nameOf(i)))) ?? [])
const skipped = computed(() => (jr.preview?.items.length ?? 0) - ready.value.length)
const changed = computed(() => jr.preview?.items.filter((i) => overridable(i.reason)) ?? [])
const unsupported = computed(() => jr.preview?.items.some((i) => i.reason === 'unsupported') ?? false)
const trashWarn = computed(() => jr.preview?.items.some((i) => i.action === 'trashOutput' && jr.named.has(nameOf(i))) ?? false)
const untruncated = computed(() => jr.preview?.total ?? 0)
const toggleNamed = (p: string, on: boolean) => (on ? jr.named.add(p) : jr.named.delete(p))
const counts = computed(() => {
  const o = jr.report?.outcomes ?? []
  const n = (s: string) => o.filter((x) => x.status === s).length
  return { done: n('done'), skipped: n('skipped'), failed: n('failed') }
})
const notDone = computed(() => jr.report?.outcomes.filter((o) => o.status !== 'done') ?? [])
const macNote = computed(() => isMac && unsupported.value)
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-40 flex h-[85vh] w-[min(860px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none">
        <header class="flex h-12 shrink-0 items-center gap-3 px-4">
          <Icon name="clock" class="size-4 text-accent" />
          <DialogTitle class="shrink-0 text-[14px] font-medium">{{ t('journal.title') }}</DialogTitle>
          <DialogDescription class="min-w-0 flex-1 truncate text-[12px] text-dim">{{ t('journal.local') }}</DialogDescription>
          <button class="h-7 rounded-md px-2 text-[12px] text-danger hover:bg-danger/10" @click="confirmClear">{{ t('journal.clear') }}</button>
          <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" :aria-label="t('common.close')">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </header>

        <!-- dry run -->
        <div v-if="jr.preview" class="flex min-h-0 flex-1 flex-col px-4 pb-4 text-[12.5px]">
          <h3 class="text-[14px] font-medium">{{ t('journal.previewTitle', { label: jr.preview.label }) }}</h3>
          <p class="mt-1 text-dim">{{ fmtDateFull(jr.preview.time) }} · {{ t('journal.previewNote') }}</p>
          <p v-if="untruncated > jr.preview.items.length && !jr.preview.blocked" class="mt-1 text-dim">{{ t('journal.recordedOnly', { n: jr.preview.items.length, total: untruncated }) }}</p>

          <div v-if="macNote" class="mt-3 flex items-center gap-3 rounded-md bg-raised p-3">
            <Icon name="info" class="size-4 shrink-0 text-accent" />
            <span class="flex-1">{{ t('journal.macRestore') }}</span>
            <button class="h-7 shrink-0 rounded-md bg-panel px-3 hover:bg-accent-soft" @click="showTrash">{{ t('history.showTrash') }}</button>
          </div>
          <div v-if="jr.preview.newer.length" class="mt-3 rounded-md bg-raised p-3" role="alert">
            <p class="flex items-center gap-2 font-medium"><Icon name="warning" class="size-4 shrink-0 text-danger" />{{ t('journal.newerTitle') }}</p>
            <ul class="mt-1 ml-6 list-disc text-dim">
              <li v-for="n in jr.preview.newer" :key="n.id">{{ fmtDateFull(n.time) }}: {{ n.label }}</li>
            </ul>
          </div>
          <div v-if="changed.length" class="mt-3 rounded-md bg-raised p-3">
            <p class="font-medium">{{ t('journal.includeTitle') }}</p>
            <label v-for="i in changed.slice(0, 200)" :key="nameOf(i)" class="mt-1 flex min-w-0 items-center gap-2">
              <input type="checkbox" :checked="jr.named.has(nameOf(i))" @change="toggleNamed(nameOf(i), ($event.target as HTMLInputElement).checked)" />
              <span class="truncate" :title="nameOf(i)">{{ nameOf(i) }}</span>
              <span class="shrink-0 text-faint">{{ tr(`journal.reason.${i.reason}`) }}</span>
            </label>
            <p v-if="trashWarn" class="mt-2 text-danger">{{ t('journal.includeTrashWarn') }}</p>
          </div>

          <ul class="mt-3 min-h-0 flex-1 overflow-auto rounded-md bg-raised/40 p-1">
            <li v-for="(i, n) in jr.preview.items" :key="n" class="flex min-w-0 items-center gap-2 rounded px-2 py-1">
              <Icon :name="i.reason && !jr.named.has(nameOf(i)) ? 'x' : 'check'" class="size-3.5 shrink-0" :class="i.reason && !jr.named.has(nameOf(i)) ? 'text-danger' : 'text-accent'" />
              <span class="shrink-0 text-dim">{{ tr(`journal.action.${i.action}`) }}</span>
              <span class="min-w-0 flex-1 truncate" :title="`${i.from} → ${i.to}`">
                <template v-if="i.from">{{ i.from }}</template><template v-if="i.from && i.to"> → </template>{{ i.to }}
              </span>
              <span v-if="i.newer" class="shrink-0 text-[11px] text-danger">{{ t('journal.newerItem') }}</span>
              <span v-if="i.reason" class="shrink-0 text-[11px] text-dim">{{ tr(`journal.reason.${i.reason}`) }}</span>
            </li>
          </ul>

          <footer class="mt-3 flex items-center gap-3">
            <span class="flex-1 text-dim" aria-live="polite">
              {{ t('journal.willDo', { count: ready.length }) }}<template v-if="skipped"> · {{ t('journal.willSkip', { count: skipped }) }}</template>
            </span>
            <button class="h-8 rounded-md bg-raised px-3 hover:bg-accent-soft" :disabled="jr.running" @click="jr.preview = null">{{ t('journal.back') }}</button>
            <button class="h-8 rounded-md bg-accent px-3 text-on-accent disabled:opacity-40" :disabled="!ready.length || jr.running" @click="runUndo">
              {{ jr.running ? t('journal.running') : t('journal.undoRun', { count: ready.length }) }}
            </button>
          </footer>
        </div>

        <!-- result -->
        <div v-else-if="jr.report" class="flex min-h-0 flex-1 flex-col px-4 pb-4 text-[12.5px]">
          <h3 class="text-[14px] font-medium">{{ t('journal.reportTitle') }}</h3>
          <p class="mt-1" aria-live="polite">{{ t('journal.reportCounts', counts) }}</p>
          <p class="mt-1 text-dim">{{ jr.report.applied ? (notDone.length ? t('journal.reportPartial') : '') : t('journal.reportNone') }}</p>
          <p v-if="jr.report.saveError" class="mt-1 text-danger">{{ t('journal.saveError', { error: jr.report.saveError }) }}</p>
          <ul class="mt-3 min-h-0 flex-1 overflow-auto">
            <li v-for="(o, n) in notDone" :key="n" class="py-1">
              <span class="block truncate" :title="o.from || o.to">{{ o.from || o.to }}</span>
              <span class="text-dim">{{ o.reason ? tr(`journal.reason.${o.reason}`) : o.detail }}</span>
            </li>
          </ul>
          <footer class="mt-3 flex justify-end"><button class="h-8 rounded-md bg-raised px-3 hover:bg-accent-soft" @click="jr.report = null">{{ t('journal.back') }}</button></footer>
        </div>

        <template v-else>
          <div class="flex shrink-0 items-center gap-2 px-4 pb-2" role="tablist">
            <button v-for="k in (['list', 'where'] as const)" :key="k" role="tab" :aria-selected="jr.tab === k" class="h-7 rounded-full px-3 text-[12px]" :class="jr.tab === k ? 'bg-accent-soft text-fg' : 'text-dim hover:bg-raised'" @click="jr.tab = k">{{ tr(`journal.tab.${k}`) }}</button>
            <template v-if="jr.tab === 'list'">
              <input v-model="jr.search" class="ml-auto h-7 w-56 rounded-md bg-raised px-2 text-[12px] outline-none focus:ring-1 focus:ring-accent/50" :placeholder="t('journal.search')" :aria-label="t('journal.search')" />
              <select v-model="jr.kind" class="h-7 rounded-md bg-raised px-2 text-[12px] outline-none" :aria-label="t('journal.kindFilter')">
                <option value="">{{ t('journal.allKinds') }}</option>
                <option v-for="k in KINDS" :key="k" :value="k">{{ tr(`journal.kind.${k}`) }}</option>
              </select>
            </template>
          </div>
          <p v-if="!settings.journal" class="mx-4 mb-2 rounded-md bg-raised px-3 py-2 text-[12px] text-dim">{{ t('journal.off') }}</p>
          <p v-if="jr.setAside" class="mx-4 mb-2 rounded-md bg-raised px-3 py-2 text-[12px] break-all text-dim">{{ t('journal.setAside', { path: jr.setAside }) }}</p>

          <!-- list by day -->
          <div v-if="jr.tab === 'list'" class="min-h-0 flex-1 overflow-auto px-4 pb-4 text-[12.5px]">
            <p v-if="!jr.rows.length && !jr.loading" class="py-10 text-center text-dim">{{ jr.all ? t('journal.noMatch') : t('journal.empty') }}</p>
            <section v-for="d in days" :key="d.key">
              <h3 class="sticky top-0 bg-panel py-1 text-[11.5px] font-medium text-dim">{{ d.label }}</h3>
              <div v-for="r in d.rows" :key="r.id" class="rounded-md hover:bg-raised/40">
                <button class="flex w-full min-w-0 items-center gap-2 px-2 py-1.5 text-left" :aria-expanded="jr.expanded === r.id" @click="expand(r.id)">
                  <span class="w-10 shrink-0 text-faint tabular-nums">{{ timeOf(r.time) }}</span>
                  <span class="w-40 shrink-0 truncate text-dim">{{ tr(`journal.kind.${r.kind}`) }}</span>
                  <span class="min-w-0 flex-1 truncate" :class="r.undone && 'line-through opacity-60'">{{ r.label }}</span>
                  <span v-if="r.status !== 'done'" class="shrink-0 rounded bg-danger/10 px-1.5 text-[11px] text-danger">{{ tr(`journal.status.${r.status}`) }}</span>
                  <span v-if="r.undone" class="shrink-0 rounded bg-raised px-1.5 text-[11px] text-dim">{{ t('journal.undoneBadge') }}</span>
                  <span v-if="r.partlyUndone" class="shrink-0 rounded bg-raised px-1.5 text-[11px] text-dim">{{ t('journal.partlyBadge') }}</span>
                  <span v-if="r.isUndo" class="shrink-0 rounded bg-raised px-1.5 text-[11px] text-dim">{{ t('journal.undoBadge') }}</span>
                </button>
                <div v-if="jr.expanded === r.id" class="px-2 pb-2">
                  <p v-if="r.error" class="mb-1 break-words text-danger">{{ r.error }}</p>
                  <ul class="max-h-64 overflow-auto">
                    <li v-for="(p, n) in jr.pairs" :key="n" class="flex min-w-0 items-center gap-2 py-0.5">
                      <span class="min-w-0 flex-1 truncate" :title="`${p.from} → ${p.to}`"><template v-if="p.from">{{ p.from }}</template><template v-if="p.from && p.to"> → </template>{{ p.to }}</span>
                      <span v-if="!p.done" class="shrink-0 text-[11px] text-faint">{{ t('journal.notDone') }}</span>
                      <template v-else>
                        <span class="shrink-0 text-[11px]" :class="nowTone(p.now)" :title="p.latest">{{ tr(`journal.now.${p.now}`) }}</span>
                        <button v-if="canReveal(p.now)" class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('journal.reveal')" :aria-label="t('journal.reveal')" @click="reveal(p.latest)"><Icon name="folder-open" class="size-3.5" /></button>
                      </template>
                    </li>
                  </ul>
                  <p v-if="r.total > jr.pairs.length && jr.pairs.length" class="mt-1 text-dim">{{ t('journal.recordedOnly', { n: jr.pairs.length, total: r.total }) }}</p>
                  <div class="mt-2">
                    <button v-if="!r.blocked" class="h-7 rounded-md bg-raised px-3 hover:bg-accent-soft" @click="startUndo(r.id)">{{ t('journal.undoEllipsis') }}</button>
                    <p v-else class="text-dim">{{ tr(`journal.blocked.${r.blocked}`) }}</p>
                  </div>
                </div>
              </div>
            </section>
            <button v-if="jr.rows.length < jr.matched" class="mx-auto mt-2 block h-7 rounded-md bg-raised px-3 hover:bg-accent-soft" @click="load(true)">{{ t('journal.more') }}</button>
          </div>

          <!-- where did it go -->
          <div v-else class="flex min-h-0 flex-1 flex-col px-4 pb-4 text-[12.5px]">
            <input v-model="jr.whereQuery" class="h-8 shrink-0 rounded-md bg-raised px-2 outline-none focus:ring-1 focus:ring-accent/50" :placeholder="t('journal.wherePlaceholder')" :aria-label="t('journal.wherePlaceholder')" />
            <p class="mt-2 text-dim">{{ t('journal.whereCaveat') }}</p>
            <div class="mt-2 min-h-0 flex-1 overflow-auto">
              <p v-if="!jr.chains" class="py-8 text-center text-dim">{{ t('journal.whereHint') }}</p>
              <p v-else-if="!jr.chains.length" class="py-8 text-center text-dim">{{ t('journal.whereNone') }}</p>
              <section v-for="(c, n) in jr.chains ?? []" :key="n" class="mb-2 rounded-md bg-raised/40 p-2">
                <p class="truncate text-dim" :title="c.start">{{ c.start }}</p>
                <p v-for="h in c.hops" :key="h.entry" class="flex min-w-0 gap-2 py-0.5">
                  <span class="shrink-0 text-faint">{{ fmtDateFull(h.time) }}</span>
                  <span class="shrink-0 text-dim">{{ tr(`journal.kind.${h.kind}`) }}</span>
                  <span class="min-w-0 truncate" :title="h.to">{{ h.to && h.to !== h.from ? `→ ${h.to}` : '' }}</span>
                </p>
                <p class="mt-1 flex min-w-0 items-center gap-2 font-medium">
                  <template v-if="c.end === 'located'">
                    <span class="shrink-0" :class="!c.exists && 'text-danger'">{{ c.exists ? t('journal.chain.located') : t('journal.chain.missing') }}:</span>
                    <span class="min-w-0 flex-1 truncate" :title="c.latest">{{ c.latest }}</span>
                    <button v-if="c.exists" class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('journal.reveal')" :aria-label="t('journal.reveal')" @click="reveal(c.latest)"><Icon name="folder-open" class="size-3.5" /></button>
                  </template>
                  <span v-else class="text-danger">{{ c.end === 'trash' ? t('journal.chain.trash') : t('journal.chain.deleted') }}</span>
                </p>
              </section>
            </div>
          </div>
        </template>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
