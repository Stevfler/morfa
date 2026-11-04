import { computed, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import * as S from '../../lib/store'
import { journalLink, journalRecord, pushUndo } from '../../lib/store/history'
import { basename, fmtSize } from '../../lib/util'
import { t } from '../../lib/i18n'

/** Mirrors `dedupe::{scan::{DupFile, Group, ScanResult}, apply::Report}` in Rust. */
export type DupFile = { path: string; size: number; mtime: number; links: number; aliases: string[]; cloud: 'mirror' | 'provider' | null; protection: 'safe' | 'review' | 'protected'; whole: string | null }
export type Group = { hash: string; size: number; reclaimable: number; files: DupFile[] }
type Skipped = { unreadable: number; special: number; symlinks: number; otherFilesystem: number; changed: number; belowMinimum: number }
type Result = { scanId: number; root: string; minSize: number; groups: Group[]; reclaimable: number; skipped: Skipped; placeholders: string[]; placeholderCount: number; elapsedMs: number; stages: { walked: number } }
export type Outcome = { path: string; status: 'done' | 'skipped' | 'failed' | 'cancelled'; reason: string | null; detail: string | null }
export type Report = { outcomes: Outcome[]; trashed: string[]; bytes: number; freed: number }
export type Keep = 'newest' | 'oldest' | 'shortest'

/** Drawn at once; helpers, counts and the safety checks cover the rest too. */
export const GROUP_LIMIT = 300
export const FILE_LIMIT = 50
const MIB = 1024 * 1024

export const dd = reactive({
  open: false,
  root: '',
  minMiB: 1,
  step: 'setup' as 'setup' | 'list' | 'plan' | 'running' | 'report',
  scanning: false,
  id: 0,
  phase: 'walk' as 'walk' | 'partial' | 'full',
  done: 0,
  total: 0,
  bytes: 0,
  current: '',
  error: '',
  result: null as Result | null,
  /** Only ever filled in by the user's click or a helper they pressed. */
  selected: new Set<string>(),
  filter: '',
  onlyChosen: false,
  report: null as Report | null,
  /** The run ended while the dialog was closed. */
  reportUnseen: false,
})

/** Can it be ticked by hand? The backend refuses protected paths whatever is sent. */
export const selectable = (f: DupFile) => f.protection !== 'protected'
/** The helpers leave out whatever deserves a look of its own. */
const helperMay = (f: DupFile) => f.protection === 'safe' && !f.cloud && !f.whole
/** Other names keep a hard-linked file on disk. */
const frees = (f: DupFile) => f.links <= 1

const everySelected = (g: Group, sel: Set<string>) => g.files.every((f) => sel.has(f.path))

export const shownGroups = computed(() => {
  const q = dd.filter.trim().toLowerCase()
  const groups = dd.result?.groups ?? []
  if (!q && !dd.onlyChosen) return groups
  return groups.filter((g) => (!q || g.files.some((f) => f.path.toLowerCase().includes(q))) && (!dd.onlyChosen || g.files.some((f) => dd.selected.has(f.path))))
})

/** Whatever is selected, shown or not. */
export const plan = computed(() => {
  const paths: string[] = []
  let bytes = 0
  let freed = 0
  let cloud = 0
  let unsafe = 0
  const wholes: { path: string; whole: string }[] = []
  for (const g of dd.result?.groups ?? []) {
    let n = 0
    for (const f of g.files) {
      if (!dd.selected.has(f.path)) continue
      n++
      paths.push(f.path)
      bytes += f.size
      if (frees(f)) freed += f.size
      if (f.cloud) cloud++
      if (f.whole) wholes.push({ path: f.path, whole: f.whole })
    }
    if (n && n >= g.files.length) unsafe++
  }
  return { paths, bytes, freed, cloud, unsafe, wholes }
})

/** Selected files no row on screen shows, because of the filter or the row limit. */
export const hiddenChosen = computed(() => {
  const drawn = new Set<string>()
  for (const g of shownGroups.value.slice(0, GROUP_LIMIT)) for (const f of g.files.slice(0, FILE_LIMIT)) drawn.add(f.path)
  return plan.value.paths.filter((p) => !drawn.has(p)).length
})

/** Never selects the last unselected copy of a group. */
export function toggle(g: Group, f: DupFile, on: boolean) {
  if (!on) return void dd.selected.delete(f.path)
  if (!selectable(f)) return
  if (g.files.every((x) => x.path === f.path || dd.selected.has(x.path))) return S.toast({ title: t('dedupe.lastCopy'), tone: 'info' }, 3000)
  dd.selected.add(f.path)
  if (f.whole) S.toast({ title: t('dedupe.wholeWarn', { name: f.whole }), tone: 'info' }, 6000)
}

const byNewest = (a: DupFile, b: DupFile) => b.mtime - a.mtime
/** An unknown time isn't the oldest. */
const byOldest = (a: DupFile, b: DupFile) => (a.mtime || Infinity) - (b.mtime || Infinity)
const byShortest = (a: DupFile, b: DupFile) => a.path.length - b.path.length || (a.path < b.path ? -1 : 1)
const SORT = { newest: byNewest, oldest: byOldest, shortest: byShortest }

/** Picks the extras in every group, shown or not, replacing the selection. One file of each stays. */
export function helper(kind: Keep) {
  dd.selected.clear()
  for (const g of dd.result?.groups ?? []) {
    // Only a plain copy can be the one that stays: no plain copy, or nothing else to pick, no selection.
    const plain = g.files.filter(helperMay)
    const keep = [...plain].sort(SORT[kind])[0]
    for (const f of plain) if (f !== keep) dd.selected.add(f.path)
  }
  dd.onlyChosen = false
}

const inFolder = (p: string, dir: string) => p.startsWith(dir.endsWith('/') || dir.endsWith('\\') ? dir : dir + (dir.includes('\\') ? '\\' : '/'))
export const folderOf = (p: string) => p.slice(0, Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'))) || p

/** Keeps what's in `dir`, at any depth. A group with no copy there is left as it is: unselected. */
export function keepIn(dir: string) {
  dd.selected.clear()
  for (const g of dd.result?.groups ?? []) {
    if (!g.files.some((f) => helperMay(f) && inFolder(f.path, dir))) continue
    for (const f of g.files) if (!inFolder(f.path, dir) && helperMay(f)) dd.selected.add(f.path)
  }
  dd.onlyChosen = false
}

export function explain(e: unknown) {
  const s = String(e)
  if (s === 'dedupe-stale' || s === 'dedupe-not-in-scan') return t('dedupe.stale')
  if (s === 'dedupe-every-copy') return t('dedupe.lastCopy')
  if (s === 'dedupe-busy') return t('dedupe.busy')
  if (s === 'dedupe-unconfirmed' || s === 'dedupe-cloud-unconfirmed' || s === 'dedupe-whole-unconfirmed') return t('dedupe.unconfirmed')
  return null
}

let unlisten: UnlistenFn | null = null

/** For the pane's folder, or a drive from the clean-up tool. */
export function openDedupe(root: string) {
  // A run is still going, or ended unseen: back to it.
  if (dd.step === 'running') return void (dd.open = true)
  // Its report stays reachable from the toast when another folder is opened instead.
  if (dd.reportUnseen && dd.report && dd.root === root) return showReport(dd.report, root)
  dd.reportUnseen = false
  if (dd.scanning) void cancelScan()
  dd.id = 0
  dd.scanning = false
  Object.assign(dd, { open: true, root, step: 'setup', result: null, report: null, error: '', filter: '', onlyChosen: false })
  dd.selected.clear()
}

function showReport(report: Report, root: string) {
  if (dd.scanning) void cancelScan()
  dd.reportUnseen = false
  dd.scanning = false
  // Results of another folder don't go with this report.
  if (dd.root !== root || dd.step === 'running') dd.result = null
  Object.assign(dd, { open: true, step: 'report', report, root })
}

export async function scan() {
  const id = S.nextId()
  Object.assign(dd, { id, scanning: true, step: 'setup', phase: 'walk', done: 0, total: 0, bytes: 0, current: '', error: '', result: null, report: null, filter: '', onlyChosen: false })
  dd.selected.clear()
  unlisten ??= await listen<{ id: number; phase: typeof dd.phase; done: number; total: number; bytes: number; current: string }>('dedupe-progress', ({ payload: p }) => {
    if (p.id === dd.id) Object.assign(dd, { phase: p.phase, done: p.done, total: p.total, bytes: p.bytes, current: p.current })
  })
  try {
    const minSize = Math.max(1, Math.round((Number(dd.minMiB) || 0) * MIB))
    const result = await invoke<Result>('dedupe_scan', { id, root: dd.root, minSize })
    if (dd.id !== id) return
    dd.result = result
    dd.step = 'list'
  } catch (e) {
    if (dd.id !== id) return
    const s = String(e)
    dd.error = s === 'cancelled' ? t('dedupe.cancelled') : S.isNotResponding(e) ? t('pane.notResponding') : s
  } finally {
    if (dd.id === id) dd.scanning = false
  }
}

export const cancelScan = () => invoke('dedupe_cancel', { id: dd.id })

export function close() {
  // The run goes on; its report comes back through a toast.
  if (dd.step === 'running') return void (dd.open = false)
  if (dd.scanning) void cancelScan()
  dd.id = 0
  dd.scanning = false
  dd.open = false
  dd.result = null
  dd.report = null
  dd.selected.clear()
}

export function confirmRemoval() {
  const p = plan.value
  if (!p.paths.length || p.unsafe) return
  S.state.confirm = {
    title: t('dedupe.confirmTitle', { count: p.paths.length, size: fmtSize(p.bytes) }),
    desc: [t('dedupe.confirmDesc'), p.cloud ? t('dedupe.confirmCloud', { count: p.cloud }) : '', p.wholes.length ? t('dedupe.confirmWhole', { names: [...new Set(p.wholes.map((w) => w.whole))].join(', ') }) : ''].filter(Boolean).join(' '),
    ok: t('dedupe.confirmOk'),
    danger: true,
    run: () => void execute(p.cloud > 0, p.wholes.length > 0),
  }
}

async function execute(confirmCloud: boolean, confirmWhole: boolean) {
  const result = dd.result
  if (!result) return
  const paths = plan.value.paths
  if (plan.value.unsafe) return
  const id = S.nextId()
  removeStarted = Date.now()
  dd.step = 'running'
  S.state.ops.push({ id, label: t('dedupe.removing', { name: basename(result.root) || result.root }), done: 0, total: 0, current: '', kind: 'dedupe' })
  try {
    const report = await invoke<Report>('dedupe_remove', { id, plan: { scanId: result.scanId, paths, confirmed: true, confirmCloud, confirmWhole } })
    forget(report.trashed)
    dd.report = report
    dd.step = 'report'
    recordUndo(report)
    if (!dd.open) announce(report)
  } catch (e) {
    dd.step = 'plan'
    if (S.isNotResponding(e)) {
      // Some files may have moved; only a new scan can say.
      Object.assign(dd, { step: 'setup', result: null, error: t('dedupe.stuck') })
      dd.selected.clear()
      return void S.toast({ title: t('dedupe.stuck'), tone: 'error' }, 10000)
    }
    const why = explain(e)
    if (why) S.toast({ title: why, tone: 'error' }, 8000)
    else S.fail(e)
  } finally {
    S.state.ops = S.state.ops.filter((o) => o.id !== id)
    S.refreshAll()
  }
}

/** Results follow what happened; a group with one file left is no longer a group. */
function forget(trashed: string[]) {
  const r = dd.result
  if (!r) return
  const gone = new Set(trashed)
  for (const g of r.groups) g.files = g.files.filter((f) => !gone.has(f.path))
  r.groups = r.groups.filter((g) => g.files.length > 1)
  for (const g of r.groups) g.reclaimable = g.size * Math.min(g.files.length - 1, g.files.filter(frees).length)
  r.groups.sort((a, b) => b.reclaimable - a.reclaimable)
  r.reclaimable = r.groups.reduce((a, g) => a + g.reclaimable, 0)
  for (const p of trashed) dd.selected.delete(p)
}

export const countOf = (r: Report, s: Outcome['status']) => r.outcomes.filter((o) => o.status === s).length

/** The dialog was closed during the run: the outcome mustn't go unseen. */
function announce(report: Report) {
  dd.reportUnseen = true
  const root = dd.root
  const failed = countOf(report, 'failed')
  S.toast(
    {
      title: t('dedupe.finished'),
      desc: reportLine(report),
      tone: failed ? 'error' : countOf(report, 'skipped') ? 'info' : 'ok',
      action: { label: t('dedupe.showReport'), run: () => showReport(report, root) },
    },
    30000,
  )
}

export const reportLine = (r: Report) => t('dedupe.reportCounts', { done: countOf(r, 'done'), skipped: countOf(r, 'skipped'), failed: countOf(r, 'failed'), cancelled: countOf(r, 'cancelled'), size: fmtSize(r.bytes) || `0 ${t('size.B')}` })

/** Back from Trash where the OS can do that; on macOS the history's message points at Finder's "Put Back". */
let removeStarted = 0
function recordUndo({ trashed, outcomes }: Report) {
  const started = removeStarted
  const status = outcomes.some((o) => o.status === 'cancelled') ? 'cancelled' : !outcomes.some((o) => o.status === 'failed') ? 'done' : trashed.length ? 'partial' : 'failed'
  if (!trashed.length) {
    if (status !== 'done') journalRecord('dedupe', t('dedupe.trashed', { count: 0 }), [], status, undefined, started)
    return
  }
  pushUndo({
    label: t('dedupe.trashed', { count: trashed.length }),
    undo: () => invoke('compare_undo', { created: [], replaced: [], trashed }),
    redo: async () => {
      throw t('dedupe.noRedo')
    },
    journal: journalLink('dedupe', t('dedupe.trashed', { count: trashed.length }), () => trashed.map((p) => [p, '']), status, undefined, started),
  })
}
