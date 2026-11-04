<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import { fmtDateFull, fmtSize, isMac } from '../../lib/util'
import { t } from '../../lib/i18n'
import {
  cmp, rows, stats, plan, CLASSES, ROW_LIMIT, scan, cancelScan, close, choose, chooseShown, allowedOps, keyOf, renamed, runPlan,
  classLabel, basisLabel, opLabel, reasonLabel, type Item, type ItemClass, type Mode, type Snap, type SyncOp,
} from './compare'

const open = computed({ get: () => cmp.open, set: (v) => !v && close() })
// The filter's button is disabled once nothing is chosen: don't strand the list on it.
watch(() => cmp.choices.size, (n) => {
  if (!n) cmp.onlyChosen = false
})
const drawn = computed(() => rows.value.slice(0, ROW_LIMIT))
const group = ref<SyncOp | ''>('')
const OPS: SyncOp[] = ['copyLeftToRight', 'copyRightToLeft', 'deleteLeft', 'deleteRight']

function toggle(c: ItemClass) {
  if (cmp.shown.has(c)) cmp.shown.delete(c)
  else cmp.shown.add(c)
}
function setMode(m: Mode) {
  if (cmp.mode === m || cmp.scanning) return
  cmp.mode = m
  void scan()
}
function applyGroup() {
  chooseShown(group.value)
  group.value = ''
}
const describe = (s: Snap | null) => {
  if (!s) return '—'
  if (s.kind === 'symlink') return `→ ${s.target ?? ''}`
  const size = s.kind === 'dir' ? `${t('compare.filesN', { count: s.files })}, ${fmtSize(s.size)}` : fmtSize(s.size)
  return `${size} · ${fmtDateFull(s.mtime)}`
}
const opOf = (i: Item) => cmp.choices.get(keyOf(i)) ?? ''
const onPick = (i: Item, e: Event) => choose(i, (e.target as HTMLSelectElement).value as SyncOp | '')
const chosenHidden = computed(() => cmp.choices.size - drawn.value.filter((i) => cmp.choices.has(keyOf(i))).length)
const hasChoices = computed(() => cmp.choices.size > 0)
const tone = (c: ItemClass) => (c === 'typeConflict' || c === 'sameSizeDifferent' || c === 'differs' ? 'text-warn' : c === 'skipped' ? 'text-faint' : 'text-dim')
const sideOf = (o: SyncOp) => t(o === 'deleteLeft' || o === 'copyRightToLeft' ? 'compare.left' : 'compare.right')
const statusCount = (s: string) => cmp.report?.outcomes.filter((o) => o.status === s).length ?? 0
const notDone = computed(() => cmp.report?.outcomes.filter((o) => o.status !== 'done' || o.reason) ?? [])
const undoNote = computed(() => {
  const r = cmp.report
  if (!r) return ''
  if (r.replaced.length + r.trashed.length === 0) return t('compare.undoCopies')
  return isMac ? t('compare.undoMac') : t('compare.undoFull')
})
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-40 flex h-[85vh] w-[90vw] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none">
        <header class="flex h-12 shrink-0 items-center gap-3 px-4">
          <Icon name="columns" class="size-4 text-accent" />
          <DialogTitle class="shrink-0 text-[14px] font-medium">{{ t('compare.title') }}</DialogTitle>
          <DialogDescription class="min-w-0 flex-1 truncate text-[12px] text-dim" :title="`${cmp.left} ↔ ${cmp.right}`">{{ cmp.left }} ↔ {{ cmp.right }}</DialogDescription>
          <div class="flex items-center gap-1 rounded-full bg-raised p-0.5 text-[12px]" role="radiogroup" :aria-label="t('compare.mode')">
            <button
              v-for="m in (['quick', 'content'] as Mode[])" :key="m" role="radio" :aria-checked="cmp.mode === m" :disabled="cmp.scanning || cmp.step === 'running'"
              :title="t(m === 'quick' ? 'compare.modeQuickTip' : 'compare.modeContentTip')"
              class="h-6 rounded-full px-3 disabled:opacity-50" :class="cmp.mode === m ? 'bg-accent-soft text-accent' : 'text-dim hover:text-fg'" @click="setMode(m)"
            >{{ t(m === 'quick' ? 'compare.modeQuick' : 'compare.modeContent') }}</button>
          </div>
          <button v-if="cmp.scanning" class="h-7 rounded-full bg-raised px-3 text-[12px] hover:bg-accent-soft" @click="cancelScan">{{ t('common.cancel') }}</button>
          <button v-else :disabled="cmp.step === 'running'" class="flex h-7 items-center gap-1.5 rounded-full bg-raised px-3 text-[12px] hover:bg-accent-soft disabled:opacity-50" @click="scan">
            <Icon name="refresh" class="size-3.5" /> {{ t('compare.rescan') }}
          </button>
          <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" :aria-label="t('common.close')">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </header>

        <!-- scanning or nothing to show -->
        <template v-if="cmp.step === 'report' && cmp.report">
          <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-2 text-[13px]" aria-live="polite">
            <h3 class="font-medium">{{ t('compare.reportTitle') }}</h3>
            <p class="text-dim tabular-nums">{{ t('compare.reportCounts', { done: statusCount('done'), skipped: statusCount('skipped'), failed: statusCount('failed'), cancelled: statusCount('cancelled'), size: fmtSize(cmp.report.bytes) }) }}</p>
            <p class="text-[12px] text-dim">{{ undoNote }}</p>
            <div v-if="cmp.report.leftovers.length" class="rounded-md bg-warn/10 p-3 text-[12px]">
              <p>{{ t('compare.leftovers') }}</p>
              <p v-for="p in cmp.report.leftovers" :key="p" class="mt-1 break-all text-dim">{{ p }}</p>
            </div>
            <ul v-if="notDone.length" class="space-y-1 text-[12px]">
              <li v-for="o in notDone" :key="o.rel + o.op" class="rounded-md bg-raised/60 px-3 py-1.5">
                <p class="truncate" :title="o.rel">{{ o.rel }}</p>
                <p class="text-[11px] text-dim">{{ opLabel(o.op) }} · {{ o.reason ? reasonLabel(o.reason) : '' }}<template v-if="o.detail"> · {{ o.detail }}</template></p>
              </li>
            </ul>
          </div>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <button class="ml-auto h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110" @click="scan">{{ t('compare.again') }}</button>
          </footer>
        </template>

        <div v-else-if="!cmp.result" class="grid flex-1 place-items-center p-6 text-center text-[13px] text-dim" aria-live="polite">
          <div v-if="cmp.scanning" class="max-w-full">
            <Icon name="loader" class="mx-auto mb-2 size-5 animate-spin" />
            <p>{{ t('compare.scanning', { count: cmp.files, n: cmp.files.toLocaleString() }) }}<template v-if="cmp.bytes"> · {{ fmtSize(cmp.bytes) }}</template></p>
            <p class="mt-1 max-w-[70vw] truncate text-[11.5px] text-faint">{{ cmp.current }}</p>
          </div>
          <p v-else>{{ cmp.error }}</p>
        </div>

        <!-- results -->
        <template v-else-if="cmp.step === 'list'">
          <div class="flex shrink-0 flex-wrap gap-1.5 px-4 pb-2" role="group" :aria-label="t('compare.filters')">
            <button
              v-for="c in CLASSES" :key="c" :aria-pressed="cmp.shown.has(c)" :disabled="!stats[c].count"
              class="flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[12px] ring-1 ring-fg/5 disabled:opacity-35"
              :class="cmp.shown.has(c) ? 'bg-accent-soft text-fg' : 'text-dim hover:bg-raised'" @click="toggle(c)"
            >
              {{ classLabel(c) }} <span class="tabular-nums text-dim">{{ stats[c].count.toLocaleString() }}<template v-if="stats[c].bytes"> · {{ fmtSize(stats[c].bytes) }}</template></span>
              <span v-if="c === 'same' && cmp.result.mode === 'quick'" class="text-warn">{{ t('compare.sameQuickNote') }}</span>
            </button>
            <button
              :aria-pressed="cmp.onlyChosen" :disabled="!cmp.choices.size"
              class="flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[12px] ring-1 ring-fg/5 disabled:opacity-35"
              :class="cmp.onlyChosen ? 'bg-accent-soft text-fg' : 'text-dim hover:bg-raised'" @click="cmp.onlyChosen = !cmp.onlyChosen"
            >{{ t('compare.onlyChosen') }} <span class="tabular-nums text-dim">{{ cmp.choices.size.toLocaleString() }}</span></button>
          </div>
          <div class="flex shrink-0 items-center gap-2 px-4 pb-2 text-[12px] text-dim">
            <span>{{ t('compare.applyShown', { count: rows.length }) }}</span>
            <select v-model="group" class="h-7 rounded-md bg-raised px-2 text-fg outline-none focus-visible:ring-2 focus-visible:ring-accent/60" :aria-label="t('compare.groupAction')">
              <option value="">{{ t('compare.op.none') }}</option>
              <option v-for="o in OPS" :key="o" :value="o">{{ opLabel(o) }}</option>
            </select>
            <button class="h-7 rounded-md bg-raised px-3 text-fg hover:bg-accent-soft disabled:opacity-40" :disabled="!rows.length" @click="applyGroup">{{ t('compare.apply') }}</button>
            <span v-if="rows.length > ROW_LIMIT" class="text-warn">{{ t('compare.setCovers', { total: rows.length.toLocaleString(), n: ROW_LIMIT.toLocaleString() }) }}</span>
            <span class="ml-auto">{{ t('compare.modeUsed', { mode: t(cmp.result.mode === 'quick' ? 'compare.modeQuick' : 'compare.modeContent') }) }}</span>
          </div>
          <div class="flex shrink-0 gap-3 px-4 pb-1 text-[11px] text-faint" aria-hidden="true">
            <span class="min-w-0 flex-1" />
            <span class="w-[22%]">{{ t('compare.left') }}</span>
            <span class="w-[22%]">{{ t('compare.right') }}</span>
            <span class="w-40 shrink-0" />
          </div>
          <ul class="min-h-0 flex-1 overflow-y-auto px-2" :aria-label="t('compare.results')">
            <li v-for="i in drawn" :key="keyOf(i)" class="flex min-h-9 items-center gap-3 rounded-md px-2 py-1 text-[12px] hover:bg-raised/60">
              <div class="min-w-0 flex-1">
                <p class="truncate" :title="i.leftRel">
                  {{ i.leftRel }}<span v-if="renamed(i)" class="text-faint"> ≈ {{ i.rightRel }}</span>
                </p>
                <p class="truncate text-[11px] text-faint">
                  <span :class="tone(i.class)">{{ classLabel(i.class) }}</span>
                  <template v-if="i.basis !== 'none'"> · {{ basisLabel(i.basis) }}</template>
                  <template v-if="i.reason"> · {{ reasonLabel(i.reason) }}</template>
                </p>
              </div>
              <span class="w-[22%] truncate text-[11px] text-dim tabular-nums" :title="describe(i.left)">{{ describe(i.left) }}</span>
              <span class="w-[22%] truncate text-[11px] text-dim tabular-nums" :title="describe(i.right)">{{ describe(i.right) }}</span>
              <select
                v-if="allowedOps(i).length" :value="opOf(i)" class="h-7 w-40 shrink-0 rounded-md bg-raised px-1.5 outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
                :class="opOf(i).startsWith('delete') ? 'text-danger' : opOf(i) ? 'text-accent' : 'text-dim'"
                :aria-label="t('compare.itemAction', { name: i.leftRel })" @change="onPick(i, $event)"
              >
                <option value="">{{ t('compare.op.none') }}</option>
                <option v-for="o in allowedOps(i)" :key="o" :value="o">{{ opLabel(o) }}</option>
              </select>
              <span v-else class="w-40 shrink-0" />
            </li>
            <li v-if="!rows.length" class="grid h-32 place-items-center text-[13px] text-dim">{{ t('compare.nothingShown') }}</li>
            <li v-if="rows.length > ROW_LIMIT" class="p-2 text-center text-[11.5px] text-faint">{{ t('compare.rowLimit', { n: ROW_LIMIT.toLocaleString(), total: rows.length.toLocaleString() }) }}</li>
            <li v-if="cmp.result.sameTruncated && cmp.shown.has('same')" class="p-2 text-center text-[11.5px] text-faint">{{ t('compare.sameLimit') }}</li>
          </ul>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <span class="text-dim">{{ t('compare.chosen', { count: cmp.choices.size }) }}<template v-if="chosenHidden"> · <span class="text-warn">{{ t('compare.chosenHidden', { count: chosenHidden }) }}</span></template></span>
            <button class="ml-auto h-8 rounded-md px-3 text-dim hover:bg-raised disabled:opacity-40" :disabled="!hasChoices" @click="cmp.choices.clear()">{{ t('compare.clear') }}</button>
            <button class="h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110 disabled:opacity-40" :disabled="!hasChoices" @click="cmp.step = 'plan'">{{ t('compare.review') }}</button>
          </footer>
        </template>

        <!-- plan -->
        <template v-else-if="cmp.step === 'plan'">
          <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-2 text-[13px]">
            <h3 class="font-medium">{{ t('compare.planTitle') }}</h3>
            <ul class="space-y-1">
              <li v-for="o in OPS" :key="o" class="flex justify-between rounded-md bg-raised/60 px-3 py-2" :class="!plan.groups[o].count && 'opacity-50'">
                <span :class="o.startsWith('delete') && plan.groups[o].count ? 'text-danger' : ''">{{ opLabel(o) }}</span>
                <span class="tabular-nums text-dim">{{ t('compare.itemsN', { count: plan.groups[o].count }) }} · {{ fmtSize(plan.groups[o].bytes) }}</span>
              </li>
            </ul>
            <p v-if="plan.replaces" class="text-[12px] text-dim">{{ t('compare.planReplaces', { count: plan.replaces }) }}</p>
            <p v-if="plan.deletes" class="text-[12px] text-danger">{{ t('compare.planDeletes', { count: plan.deletes }) }}</p>
            <label v-if="plan.newer" class="flex items-start gap-2 rounded-md bg-warn/10 p-3 text-[12px]">
              <input v-model="cmp.replaceNewer" type="checkbox" class="mt-0.5 size-3.5 accent-[var(--color-accent)]" />
              <span>{{ t('compare.planNewer', { count: plan.newer }) }}</span>
            </label>
            <ul v-if="plan.newer" class="space-y-0.5 text-[11.5px] text-dim">
              <li v-for="a in plan.newerItems.slice(0, 20)" :key="keyOf(a)" class="truncate">{{ a.leftRel }}</li>
              <li v-if="plan.newer > 20" class="text-faint">{{ t('compare.more', { n: (plan.newer - 20).toLocaleString() }) }}</li>
            </ul>
            <template v-for="l in [{ title: t('compare.deleteList'), items: plan.deleteList, cls: 'text-danger' }, { title: t('compare.replaceList'), items: plan.replaceList, cls: 'text-fg' }]" :key="l.title">
              <div v-if="l.items.length">
                <h4 class="mb-1 text-[12px]" :class="l.cls">{{ l.title }} ({{ l.items.length.toLocaleString() }})</h4>
                <ul class="max-h-40 space-y-0.5 overflow-y-auto rounded-md bg-raised/60 p-2 text-[11.5px] text-dim" tabindex="0">
                  <li v-for="x in l.items" :key="x.op + x.rel" class="break-all">{{ sideOf(x.op) }}: {{ x.rel }}</li>
                </ul>
              </div>
            </template>
            <p v-if="plan.held" class="text-[12px] text-warn">{{ t('compare.planHeld', { count: plan.held }) }}</p>
            <p class="text-[12px] text-dim">{{ t('compare.planRevalidate') }}</p>
          </div>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <button class="h-8 rounded-md px-3 text-dim hover:bg-raised" @click="cmp.step = 'list'">{{ t('compare.back') }}</button>
            <button class="ml-auto h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110 disabled:opacity-40" :disabled="!plan.actions.length" @click="runPlan">
              {{ t('compare.run', { count: plan.actions.length }) }}
            </button>
          </footer>
        </template>

        <!-- running: progress and cancel live in the op row -->
        <div v-else-if="cmp.step === 'running'" class="grid flex-1 place-items-center text-[13px] text-dim" aria-live="polite">
          <p><Icon name="loader" class="mr-2 inline size-4 animate-spin" />{{ t('compare.running') }}</p>
        </div>

      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
