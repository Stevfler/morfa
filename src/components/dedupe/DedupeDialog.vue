<script setup lang="ts">
import { computed, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener'
import Icon from '../Icon.vue'
import * as S from '../../lib/store'
import { canQuickLook, fmtDateFull, fmtSize, isMac } from '../../lib/util'
import { t, type MsgKey } from '../../lib/i18n'
import {
  dd, plan, shownGroups, hiddenChosen, GROUP_LIMIT, FILE_LIMIT, scan, cancelScan, close, toggle, helper, keepIn, folderOf, selectable, confirmRemoval, reportLine,
  type DupFile, type Keep,
} from './dedupe'

// Quick Look sits on top, and its file stays set through its closing animation: a click or Esc meant
// for it must not reach this dialog too, or the pane's shortcuts would be live behind it.
const looking = () => S.state.quickLook || !!S.state.quickLookFile
const open = computed({ get: () => dd.open, set: (v) => !v && !looking() && close() })
function holdForLook(e: Event) {
  if (!looking()) return
  e.preventDefault()
  ;(e as CustomEvent<{ originalEvent?: Event } | undefined>).detail?.originalEvent?.stopPropagation()
}
const HELPERS: Keep[] = ['newest', 'oldest', 'shortest']
const drawn = computed(() => shownGroups.value.slice(0, GROUP_LIMIT))
const size = (n: number) => fmtSize(n) || `0 ${t('size.B')}`
const notDone = computed(() => dd.report?.outcomes.filter((o) => o.status !== 'done') ?? [])
const phaseText = computed(() => t(`dedupe.phase.${dd.phase}` as MsgKey, { n: dd.done.toLocaleString(), total: dd.total.toLocaleString() }))
const percent = computed(() => (dd.phase !== 'walk' && dd.total ? Math.min(100, Math.round((dd.done / dd.total) * 100)) : null))
const skippedNote = computed(() => {
  const s = dd.result?.skipped
  if (!s) return ''
  return t('dedupe.skippedNote', { unreadable: s.unreadable, changed: s.changed, other: s.otherFilesystem, links: s.symlinks, special: s.special })
})
const hasSkips = computed(() => {
  const s = dd.result?.skipped
  return !!s && s.unreadable + s.changed + s.otherFilesystem + s.symlinks + s.special > 0
})

// Nothing left to show with the Chosen filter on, and its button is disabled: don't strand the list.
watch(() => dd.selected.size, (n) => {
  if (!n) dd.onlyChosen = false
})

const nameOf = (p: string) => p.slice(Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\')) + 1)
function asEntry(f: DupFile): S.Entry {
  const name = nameOf(f.path)
  const dot = name.lastIndexOf('.')
  return { name, path: f.path, isDir: false, isSymlink: false, size: f.size, modified: f.mtime, ext: dot > 0 ? name.slice(dot + 1).toLowerCase() : '', hidden: name.startsWith('.') }
}
const look = (f: DupFile) => {
  S.state.quickLookFile = asEntry(f)
  S.state.quickLook = true
}
const undoNote = computed(() => (!dd.report?.trashed.length ? '' : isMac ? t('dedupe.undoMac') : t('dedupe.undoFull')))
const reason = (r: string | null) => (r ? t(`dedupe.reason.${r}` as MsgKey) : '')
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-40 flex h-[85vh] w-[90vw] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none" @interact-outside="holdForLook" @pointer-down-outside="holdForLook" @focus-outside="holdForLook" @escape-key-down="holdForLook">
        <header class="flex h-12 shrink-0 items-center gap-3 px-4">
          <Icon name="copy" class="size-4 text-accent" />
          <DialogTitle class="shrink-0 text-[14px] font-medium">{{ t('dedupe.title') }}</DialogTitle>
          <DialogDescription class="min-w-0 flex-1 truncate text-[12px] text-dim" :title="dd.root">{{ dd.root }}</DialogDescription>
          <button v-if="dd.scanning" class="h-7 rounded-full bg-raised px-3 text-[12px] hover:bg-accent-soft" @click="cancelScan">{{ t('common.cancel') }}</button>
          <button v-else-if="dd.step === 'list' || dd.step === 'report'" class="flex h-7 items-center gap-1.5 rounded-full bg-raised px-3 text-[12px] hover:bg-accent-soft" @click="scan">
            <Icon name="refresh" class="size-3.5" /> {{ t('dedupe.rescan') }}
          </button>
          <DialogClose class="grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" :aria-label="t('common.close')">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </header>

        <!-- setup and progress -->
        <div v-if="dd.step === 'setup'" class="grid flex-1 place-items-center p-6 text-center text-[13px] text-dim" aria-live="polite">
          <div v-if="dd.scanning" class="max-w-full">
            <Icon name="loader" class="mx-auto mb-2 size-5 animate-spin" />
            <p class="tabular-nums">{{ phaseText }}<template v-if="percent !== null"> · {{ percent }}%</template></p>
            <p class="mt-1 max-w-[70vw] truncate text-[11.5px] text-faint">{{ dd.current }}</p>
          </div>
          <form v-else class="space-y-3" @submit.prevent="scan">
            <p class="max-w-md">{{ t('dedupe.intro') }}</p>
            <p v-if="dd.error" class="text-warn">{{ dd.error }}</p>
            <label class="flex items-center justify-center gap-2">
              {{ t('dedupe.minSize') }}
              <input v-model.number="dd.minMiB" type="number" min="0.001" step="any" class="h-8 w-24 rounded-md bg-raised px-2 text-right text-fg tabular-nums outline-none focus-visible:ring-2 focus-visible:ring-accent/60" />
              {{ t('dedupe.mib') }}
            </label>
            <button type="submit" class="h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110">{{ t('dedupe.start') }}</button>
          </form>
        </div>

        <!-- report -->
        <template v-else-if="dd.step === 'report' && dd.report">
          <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-2 text-[13px]" aria-live="polite">
            <h3 class="font-medium">{{ t('dedupe.reportTitle') }}</h3>
            <p class="text-dim tabular-nums">{{ reportLine(dd.report) }}</p>
            <p v-if="dd.report.freed !== dd.report.bytes" class="text-[12px] text-dim">{{ t('dedupe.freedNote', { size: size(dd.report.freed) }) }}</p>
            <p class="text-[12px] text-dim">{{ undoNote }}</p>
            <ul v-if="notDone.length" class="space-y-1 text-[12px]">
              <li v-for="o in notDone" :key="o.path" class="rounded-md bg-raised/60 px-3 py-1.5">
                <p class="truncate" :title="o.path">{{ o.path }}</p>
                <p class="text-[11px] text-dim">{{ reason(o.reason) }}<template v-if="o.detail"> · {{ o.detail }}</template></p>
              </li>
            </ul>
          </div>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <button v-if="dd.result" class="ml-auto h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110" @click="dd.step = 'list'">{{ t('dedupe.backToResults') }}</button>
          </footer>
        </template>

        <!-- running: progress and cancel live in the op row -->
        <div v-else-if="dd.step === 'running'" class="grid flex-1 place-items-center text-[13px] text-dim" aria-live="polite">
          <p><Icon name="loader" class="mr-2 inline size-4 animate-spin" />{{ t('dedupe.running') }}</p>
        </div>

        <!-- review -->
        <template v-else-if="dd.step === 'plan'">
          <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-2 text-[13px]">
            <h3 class="font-medium">{{ t('dedupe.planTitle', { count: plan.paths.length, size: size(plan.bytes) }) }}</h3>
            <p v-if="plan.freed !== plan.bytes" class="text-[12px] text-dim">{{ t('dedupe.freedNote', { size: size(plan.freed) }) }}</p>
            <p v-if="plan.cloud" class="rounded-md bg-warn/10 p-3 text-[12px]">{{ t('dedupe.planCloud', { count: plan.cloud }) }}</p>
            <p class="text-[12px] text-dim">{{ t('dedupe.planRevalidate') }}</p>
            <div v-if="plan.wholes.length" class="rounded-md bg-danger/10 p-3 text-[12px]">
              <p class="mb-1 text-danger">{{ t('dedupe.planWhole', { count: plan.wholes.length }) }}</p>
              <ul class="max-h-40 space-y-0.5 overflow-y-auto" tabindex="0">
                <li v-for="w in plan.wholes" :key="w.path" class="break-all"><span class="font-medium">{{ w.whole }}</span>: {{ w.path }}</li>
              </ul>
            </div>
            <ul class="max-h-[50vh] space-y-0.5 overflow-y-auto rounded-md bg-raised/60 p-2 text-[11.5px] text-dim" tabindex="0">
              <li v-for="p in plan.paths.slice(0, 2000)" :key="p" class="break-all">{{ p }}</li>
              <li v-if="plan.paths.length > 2000" class="text-faint">{{ t('dedupe.more', { n: (plan.paths.length - 2000).toLocaleString() }) }}</li>
            </ul>
          </div>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <button class="h-8 rounded-md px-3 text-dim hover:bg-raised" @click="dd.step = 'list'">{{ t('dedupe.back') }}</button>
            <button class="ml-auto flex h-8 items-center gap-1.5 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110 disabled:opacity-40" :disabled="!plan.paths.length || !!plan.unsafe" @click="confirmRemoval">
              <Icon name="trash" class="size-3.5" /> {{ t('dedupe.moveToTrash') }}
            </button>
          </footer>
        </template>

        <!-- results -->
        <template v-else-if="dd.result">
          <div class="flex shrink-0 flex-wrap items-center gap-2 px-4 pb-2 text-[12px] text-dim">
            <span class="tabular-nums">{{ t('dedupe.summary', { count: dd.result.groups.length, size: size(dd.result.reclaimable) }) }}</span>
            <span class="ml-auto">{{ t('dedupe.keepLabel') }}</span>
            <button v-for="h in HELPERS" :key="h" class="h-7 rounded-md bg-raised px-3 text-fg hover:bg-accent-soft disabled:opacity-40" :disabled="!dd.result.groups.length" :title="t('dedupe.helperTip')" @click="helper(h)">{{ t(`dedupe.keep.${h}` as MsgKey) }}</button>
          </div>
          <div class="flex shrink-0 items-center gap-2 px-4 pb-2 text-[12px]">
            <input v-model="dd.filter" :placeholder="t('dedupe.filter')" :aria-label="t('dedupe.filter')" class="h-7 min-w-0 flex-1 rounded-md bg-raised px-2 outline-none focus-visible:ring-2 focus-visible:ring-accent/60" />
            <button :aria-pressed="dd.onlyChosen" :disabled="!dd.selected.size" class="h-7 rounded-full px-2.5 ring-1 ring-fg/5 disabled:opacity-35" :class="dd.onlyChosen ? 'bg-accent-soft text-fg' : 'text-dim hover:bg-raised'" @click="dd.onlyChosen = !dd.onlyChosen">{{ t('dedupe.onlyChosen') }}</button>
          </div>
          <p v-if="dd.result.placeholderCount" class="mx-4 mb-2 shrink-0 rounded-md bg-warn/10 px-3 py-1.5 text-[12px]" :title="dd.result.placeholders.join('\n')">{{ t('dedupe.placeholders', { count: dd.result.placeholderCount }) }}</p>
          <p v-if="hasSkips" class="mx-4 mb-2 shrink-0 text-[11.5px] text-faint">{{ skippedNote }}</p>

          <div class="min-h-0 flex-1 space-y-2 overflow-y-auto px-3" :aria-label="t('dedupe.results')">
            <section v-for="g in drawn" :key="g.hash" class="rounded-md bg-raised/40 p-2">
              <h4 class="flex gap-2 px-1 pb-1 text-[11.5px] text-dim tabular-nums">
                <span>{{ t('dedupe.groupHead', { count: g.files.length, size: size(g.size) }) }}</span>
                <span class="ml-auto">{{ t('dedupe.groupFrees', { size: size(g.reclaimable) }) }}</span>
              </h4>
              <ul>
                <li v-for="f in g.files.slice(0, FILE_LIMIT)" :key="f.path" class="group flex min-h-8 items-center gap-2 rounded px-1 text-[12px] hover:bg-raised/60">
                  <input
                    type="checkbox" class="size-3.5 shrink-0 accent-[var(--color-danger)]" :checked="dd.selected.has(f.path)" :disabled="!selectable(f)"
                    :aria-label="t('dedupe.selectFile', { name: f.path })" @change="(e) => { toggle(g, f, (e.target as HTMLInputElement).checked); (e.target as HTMLInputElement).checked = dd.selected.has(f.path) }"
                  />
                  <span class="min-w-0 flex-1 truncate" :class="dd.selected.has(f.path) && 'text-danger'" :title="f.path">{{ f.path }}</span>
                  <span v-if="f.protection === 'protected'" class="shrink-0 text-[11px] text-faint" :title="t('dedupe.tag.protectedTip')">{{ t('dedupe.tag.protected') }}</span>
                  <span v-else-if="f.protection === 'review'" class="shrink-0 text-[11px] text-warn" :title="t('dedupe.tag.reviewTip')">{{ t('dedupe.tag.review') }}</span>
                  <span v-if="f.whole" class="shrink-0 text-[11px] text-danger" :title="t('dedupe.wholeWarn', { name: f.whole })">{{ t('dedupe.tag.whole', { name: f.whole }) }}</span>
                  <span v-if="f.cloud" class="shrink-0 text-[11px] text-warn" :title="t('dedupe.tag.cloudTip')">{{ t('dedupe.tag.cloud') }}</span>
                  <span v-if="f.links > 1" class="shrink-0 text-[11px] text-faint" :title="[t('dedupe.tag.linkedTip'), ...f.aliases].join('\n')">{{ t('dedupe.tag.linked', { n: f.links }) }}</span>
                  <span class="shrink-0 whitespace-nowrap text-right text-[11px] text-dim tabular-nums">{{ fmtDateFull(f.mtime) }}</span>
                  <button class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('dedupe.keepFolder', { path: folderOf(f.path) })" :aria-label="t('dedupe.keepFolder', { path: folderOf(f.path) })" @click="keepIn(folderOf(f.path))"><Icon name="folder" class="size-3.5" /></button>
                  <button v-if="canQuickLook(asEntry(f))" class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('cmd.quicklook')" :aria-label="t('cmd.quicklook')" @click="look(f)"><Icon name="quicklook" class="size-3.5" /></button>
                  <button class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('shell.openDefault')" :aria-label="t('shell.openDefault')" @click="openPath(f.path).catch(S.fail)"><Icon name="external-link" class="size-3.5" /></button>
                  <button class="grid size-6 shrink-0 place-items-center rounded text-dim hover:bg-raised hover:text-fg" :title="t('dedupe.reveal')" :aria-label="t('dedupe.reveal')" @click="revealItemInDir(f.path).catch(S.fail)"><Icon name="folder-open" class="size-3.5" /></button>
                </li>
                <li v-if="g.files.length > FILE_LIMIT" class="px-1 py-1 text-[11.5px] text-faint">{{ t('dedupe.more', { n: (g.files.length - FILE_LIMIT).toLocaleString() }) }}</li>
              </ul>
            </section>
            <p v-if="!shownGroups.length" class="grid h-32 place-items-center text-[13px] text-dim">{{ dd.result.groups.length ? t('dedupe.nothingShown') : t('dedupe.none') }}</p>
            <p v-if="shownGroups.length > GROUP_LIMIT" class="p-2 text-center text-[11.5px] text-faint">{{ t('dedupe.rowLimit', { n: GROUP_LIMIT.toLocaleString(), total: shownGroups.length.toLocaleString() }) }}</p>
          </div>
          <footer class="flex h-13 shrink-0 items-center gap-2 px-4 text-[12.5px]">
            <span class="text-dim tabular-nums">{{ t('dedupe.chosen', { count: plan.paths.length, size: size(plan.bytes) }) }}<template v-if="hiddenChosen"> · <span class="text-warn">{{ t('dedupe.chosenHidden', { count: hiddenChosen }) }}</span></template></span>
            <button class="ml-auto h-8 rounded-md px-3 text-dim hover:bg-raised disabled:opacity-40" :disabled="!dd.selected.size" @click="dd.selected.clear()">{{ t('dedupe.clear') }}</button>
            <button class="h-8 rounded-md bg-accent px-4 font-medium text-on-accent hover:brightness-110 disabled:opacity-40" :disabled="!plan.paths.length || !!plan.unsafe" @click="dd.step = 'plan'">{{ t('dedupe.review') }}</button>
          </footer>
        </template>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
