import { computed, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import * as S from '../../lib/store'
import { journalLink, journalRecord, pushUndo } from '../../lib/store/history'
import { basename, fmtSize } from '../../lib/util'
import { t, type MsgKey } from '../../lib/i18n'

/** Mirrors `compare::{Snap, scan::Item, scan::CompareResult, apply::SyncReport}` in Rust. */
export type Snap = { kind: 'file' | 'dir' | 'symlink' | 'special'; size: number; mtime: number; files: number; errors: number; target: string | null }
export type ItemClass = 'onlyLeft' | 'onlyRight' | 'newerLeft' | 'newerRight' | 'differs' | 'sameSizeDifferent' | 'typeConflict' | 'same' | 'skipped'
export type Basis = 'presence' | 'sizeTime' | 'sha256' | 'target' | 'type' | 'none'
export type Item = { leftRel: string; rightRel: string; class: ItemClass; basis: Basis; reason: string | null; left: Snap | null; right: Snap | null }
export type Mode = 'quick' | 'content'
type Result = { scanId: number; leftRoot: string; rightRoot: string; mode: Mode; items: Item[]; sameCount: number; sameBytes: number; sameTruncated: boolean; elapsedMs: number }
export type SyncOp = 'copyLeftToRight' | 'copyRightToLeft' | 'deleteLeft' | 'deleteRight'
type Outcome = { rel: string; op: SyncOp; status: 'done' | 'skipped' | 'failed' | 'cancelled'; reason: string | null; detail: string | null }
type Report = { outcomes: Outcome[]; created: string[]; replaced: string[]; trashed: string[]; bytes: number; leftovers: string[] }

export const CLASSES: ItemClass[] = ['onlyLeft', 'onlyRight', 'newerLeft', 'newerRight', 'differs', 'sameSizeDifferent', 'typeConflict', 'skipped', 'same']
/** Rows drawn at once; the filters narrow down the rest. */
export const ROW_LIMIT = 2000

export const cmp = reactive({
  open: false,
  left: '',
  right: '',
  mode: 'quick' as Mode,
  scanning: false,
  id: 0,
  files: 0,
  bytes: 0,
  current: '',
  error: '',
  result: null as Result | null,
  shown: new Set<ItemClass>(CLASSES.filter((c) => c !== 'same')),
  /** Show only the rows with an action chosen, whatever the class filters say. */
  onlyChosen: false,
  /** Only what the user chose. Never filled in by anything else, and never with a delete. */
  choices: new Map<string, SyncOp>(),
  step: 'list' as 'list' | 'plan' | 'running' | 'report',
  replaceNewer: false,
  report: null as Report | null,
})

export const keyOf = (i: Item) => `${i.leftRel}\0${i.rightRel}`
export const renamed = (i: Item) => !!i.left && !!i.right && i.leftRel !== i.rightRel
const sizeOf = (i: Item) => Math.max(i.left?.size ?? 0, i.right?.size ?? 0)

export const rows = computed(() => (cmp.result?.items ?? []).filter((i) => (cmp.onlyChosen ? cmp.choices.has(keyOf(i)) : cmp.shown.has(i.class))))

export const stats = computed(() => {
  const by = Object.fromEntries(CLASSES.map((c) => [c, { count: 0, bytes: 0 }])) as Record<ItemClass, { count: number; bytes: number }>
  for (const i of cmp.result?.items ?? []) {
    if (i.class === 'same') continue
    by[i.class].count++
    by[i.class].bytes += sizeOf(i)
  }
  by.same = { count: cmp.result?.sameCount ?? 0, bytes: cmp.result?.sameBytes ?? 0 }
  return by
})

/** Which actions make sense for an item. */
export function allowedOps(i: Item): SyncOp[] {
  if (i.class === 'same' || i.class === 'skipped') return []
  const ops: SyncOp[] = []
  if (i.left) ops.push('copyLeftToRight')
  if (i.right) ops.push('copyRightToLeft')
  if (i.left) ops.push('deleteLeft')
  if (i.right) ops.push('deleteRight')
  return ops
}

export function choose(i: Item, op: SyncOp | '') {
  if (op && allowedOps(i).includes(op)) cmp.choices.set(keyOf(i), op)
  else cmp.choices.delete(keyOf(i))
}

/** A group choice: the rows on show, where that action applies. */
export function chooseShown(op: SyncOp | '') {
  for (const i of rows.value) choose(i, op)
}

/** Same rule as `apply::needs_choice`: replacing something newer, or of another type. */
export function replacesNewer(i: Item, op: SyncOp) {
  const [src, dst] = op === 'copyLeftToRight' ? [i.left, i.right] : op === 'copyRightToLeft' ? [i.right, i.left] : [null, null]
  return !!src && !!dst && (src.kind !== dst.kind || dst.mtime > src.mtime || dst.mtime === 0 || src.mtime === 0)
}

export const plan = computed(() => {
  const groups = { copyLeftToRight: { count: 0, bytes: 0 }, copyRightToLeft: { count: 0, bytes: 0 }, deleteLeft: { count: 0, bytes: 0 }, deleteRight: { count: 0, bytes: 0 } }
  const actions: (Item & { op: SyncOp; replaceNewer: boolean })[] = []
  const newerItems: Item[] = []
  const deleteList: { rel: string; op: SyncOp }[] = []
  const replaceList: { rel: string; op: SyncOp }[] = []
  let replaces = 0
  let newer = 0
  let held = 0
  for (const i of cmp.result?.items ?? []) {
    const op = cmp.choices.get(keyOf(i))
    if (!op) continue
    const isNewer = replacesNewer(i, op)
    if (isNewer) {
      newer++
      newerItems.push(i)
    }
    if (isNewer && !cmp.replaceNewer) {
      held++
      continue
    }
    const copying = op.startsWith('copy')
    const src = op === 'copyLeftToRight' || op === 'deleteLeft' ? i.left : i.right
    groups[op].count++
    groups[op].bytes += src?.size ?? 0
    if (copying && i.left && i.right) {
      replaces++
      replaceList.push({ rel: op === 'copyLeftToRight' ? i.rightRel : i.leftRel, op })
    }
    if (!copying) deleteList.push({ rel: op === 'deleteLeft' ? i.leftRel : i.rightRel, op })
    actions.push({ ...i, op, replaceNewer: isNewer })
  }
  const deletes = groups.deleteLeft.count + groups.deleteRight.count
  return { groups, actions, replaces, newer, newerItems, deleteList, replaceList, held, deletes }
})

export function explain(e: unknown) {
  const s = String(e)
  if (s === 'compare-same') return t('compare.refuseSame')
  if (s === 'compare-nested') return t('compare.refuseNested')
  if (s === 'compare-changed') return t('compare.rootsChanged')
  if (s === 'compare-stale') return t('compare.stale')
  return null
}

let unlisten: UnlistenFn | null = null

/** Palette command: both panes of the split, both real folders. */
export async function openCompare() {
  // A sync is still going: back to it, it can't be mixed with a new comparison.
  if (cmp.step === 'running') return void (cmp.open = true)
  const panes = S.tab().panes
  if (panes.length < 2) return S.toast({ title: t('compare.needSplit'), tone: 'info' }, 3000)
  const [left, right] = [panes[0].path, panes[1].path]
  if (S.isSpecial(left) || S.isSpecial(right)) return S.toast({ title: t('compare.needFolders'), tone: 'info' }, 3000)
  try {
    await invoke('compare_check', { left, right })
  } catch (e) {
    const why = explain(e)
    return why ? void S.toast({ title: why, tone: 'info' }, 4000) : S.fail(e)
  }
  Object.assign(cmp, { open: true, left, right, result: null, error: '', step: 'list', report: null, onlyChosen: false })
  cmp.choices.clear()
  void scan()
}

export async function scan() {
  const id = S.nextId()
  Object.assign(cmp, { id, scanning: true, files: 0, bytes: 0, current: '', error: '', result: null, step: 'list', report: null, replaceNewer: false, onlyChosen: false })
  cmp.choices.clear()
  unlisten ??= await listen<{ id: number; files: number; bytes: number; current: string }>('compare-progress', ({ payload: p }) => {
    if (p.id === cmp.id) Object.assign(cmp, { files: p.files, bytes: p.bytes, current: p.current })
  })
  try {
    const result = await invoke<Result>('compare_scan', { id, left: cmp.left, right: cmp.right, mode: cmp.mode })
    if (cmp.id === id) cmp.result = result
  } catch (e) {
    if (cmp.id !== id) return
    const s = String(e)
    cmp.error = s === 'cancelled' ? t('compare.cancelled') : (explain(e) ?? (S.isNotResponding(e) ? t('pane.notResponding') : s))
  } finally {
    if (cmp.id === id) cmp.scanning = false
  }
}

export const cancelScan = () => invoke('compare_cancel', { id: cmp.id })

export function close() {
  // The run goes on; its report comes back through a toast.
  if (cmp.step === 'running') return void (cmp.open = false)
  if (cmp.scanning) void cancelScan()
  cmp.id = 0
  cmp.scanning = false
  cmp.open = false
  cmp.result = null
  cmp.choices.clear()
}

/** Asks once more if anything would go to Trash as a delete. */
export function runPlan() {
  const p = plan.value
  if (!p.actions.length) return
  if (!p.deletes) return void execute(false)
  S.state.confirm = {
    title: t('compare.confirmDeleteTitle', { count: p.deletes }),
    desc: t('compare.confirmDeleteDesc', { left: p.groups.deleteLeft.count, right: p.groups.deleteRight.count, size: fmtSize(p.groups.deleteLeft.bytes + p.groups.deleteRight.bytes) }),
    ok: t('compare.confirmDeleteOk'),
    danger: true,
    run: () => void execute(true),
  }
}

async function execute(confirmDeletes: boolean) {
  const result = cmp.result
  if (!result) return
  const id = S.nextId()
  syncStarted = Date.now()
  cmp.step = 'running'
  S.state.ops.push({ id, label: t('compare.syncing', { left: basename(result.leftRoot), right: basename(result.rightRoot) }), done: 0, total: 0, current: '', kind: 'compare' })
  const actions = plan.value.actions.map(({ leftRel, rightRel, op, left, right, replaceNewer }) => ({ leftRel, rightRel, op, left, right, replaceNewer }))
  try {
    const report = await invoke<Report>('compare_sync', { id, plan: { scanId: result.scanId, leftRoot: result.leftRoot, rightRoot: result.rightRoot, actions, confirmDeletes } })
    cmp.report = report
    cmp.step = 'report'
    recordUndo(report)
    if (!cmp.open) announce(report)
  } catch (e) {
    cmp.step = 'plan'
    const why = explain(e)
    if (why) S.toast({ title: why, tone: 'error' }, 8000)
    else S.fail(e)
  } finally {
    S.state.ops = S.state.ops.filter((o) => o.id !== id)
    S.refreshAll()
  }
}

/** The dialog was closed during the run: the outcome mustn't go unseen. */
function announce(report: Report) {
  const n = (s: Outcome['status']) => report.outcomes.filter((o) => o.status === s).length
  const bad = n('failed') + report.leftovers.length
  S.toast(
    {
      title: t('compare.finished'),
      desc: t('compare.reportCounts', { done: n('done'), skipped: n('skipped'), failed: n('failed'), cancelled: n('cancelled'), size: fmtSize(report.bytes) }),
      tone: bad ? 'error' : n('skipped') ? 'info' : 'ok',
      action: { label: t('compare.showReport'), run: () => Object.assign(cmp, { open: true, step: 'report', report }) },
    },
    30000,
  )
}

/**
 * The backend brings originals back before it trashes the copies that replaced them. Where the OS can't
 * restore (macOS) those stay as they are and the history's message points at Finder's "Put Back".
 * A redo would need a fresh scan.
 */
let syncStarted = 0
function recordUndo({ created, replaced, trashed, outcomes }: Report) {
  const started = syncStarted
  const n = created.length + replaced.length + trashed.length
  const cancelled = outcomes.some((o) => o.status === 'cancelled')
  const failed = outcomes.some((o) => o.status === 'failed')
  const status = cancelled ? 'cancelled' : !failed ? 'done' : n ? 'partial' : 'failed'
  const pairs: [string, string][] = [...created.map((p): [string, string] => ['', p]), ...replaced.map((p): [string, string] => ['', p]), ...trashed.map((p): [string, string] => [p, ''])]
  if (!n) {
    if (status !== 'done') journalRecord('compare', t('compare.synced', { count: outcomes.length }), [], status, undefined, started)
    return
  }
  pushUndo({
    label: t('compare.synced', { count: n }),
    undo: () => invoke('compare_undo', { created, replaced, trashed }),
    redo: async () => {
      throw t('compare.noRedo')
    },
    journal: journalLink('compare', t('compare.synced', { count: n }), () => pairs, status, undefined, started),
  })
}

export const classLabel = (c: ItemClass) => t(`compare.class.${c}` as MsgKey)
export const basisLabel = (b: Basis) => t(`compare.basis.${b}` as MsgKey)
export const opLabel = (o: SyncOp) => t(`compare.op.${o}` as MsgKey)
export const reasonLabel = (r: string) => t(`compare.reason.${r}` as MsgKey)
