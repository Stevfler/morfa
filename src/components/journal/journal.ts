import { reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import * as S from '../../lib/store'
import { forgetUndoOf, stackUndoRunning, whileJournalUndo } from '../../lib/store/history'
import type { JournalKind, JournalStatus } from '../../lib/store/types'
import { settings } from '../../lib/settings'
import { t } from '../../lib/i18n'

/** Mirrors `journal.rs` and `journal::{chain, undo}`. */
export type Row = { partlyUndone: boolean; id: number; time: number; kind: JournalKind; label: string; status: JournalStatus; error: string | null; total: number; sample: [string, string][]; undone: boolean; isUndo: boolean; blocked: string | null }
export type Now = 'here' | 'moved' | 'gone' | 'trash' | 'deleted'
export type PairNow = { from: string; to: string; done: boolean; latest: string; now: Now }
export type Hop = { entry: number; time: number; kind: JournalKind; from: string; to: string }
export type Chain = { start: string; latest: string; end: 'located' | 'trash' | 'deleted'; hops: Hop[]; exists: boolean }
export type Action = 'moveBack' | 'trashOutput' | 'restore'
export type Reason = 'metadata' | 'protected' | 'gone' | 'changed' | 'unverified' | 'occupied' | 'symlinkParent' | 'notInTrash' | 'unsupported'
export type Item = { action: Action; from: string; to: string; reason: Reason | null; newer: boolean }
export type Newer = { id: number; time: number; kind: JournalKind; label: string }
export type Preview = { id: number; label: string; kind: JournalKind; time: number; blocked: string | null; items: Item[]; newer: Newer[]; total: number }
export type Outcome = { from: string; to: string; status: 'done' | 'skipped' | 'failed'; reason: Reason | null; detail: string | null }
export type Report = { outcomes: Outcome[]; applied: boolean; undoId: number | null; saveError: string | null }

const PAGE = 200
export const KINDS: JournalKind[] = ['move', 'copy', 'rename', 'batchRename', 'trash', 'delete', 'create', 'extract', 'compress', 'convert', 'compare', 'dedupe', 'restore']

export const jr = reactive({
  open: false,
  tab: 'list' as 'list' | 'where',
  search: '',
  kind: '' as JournalKind | '',
  rows: [] as Row[],
  matched: 0,
  all: 0,
  setAside: null as string | null,
  loading: false,
  expanded: null as number | null,
  pairs: [] as PairNow[],
  whereQuery: '',
  chains: null as Chain[] | null,
  preview: null as Preview | null,
  /** Paths the user ticked although they changed. */
  named: new Set<string>(),
  running: false,
  report: null as Report | null,
})

export const overridable = (r: Reason | null) => r === 'changed' || r === 'metadata' || r === 'unverified'
/** The path the backend's `named` set goes by. */
export const nameOf = (i: Item) => (i.action === 'restore' ? i.to : i.from)

export function explain(e: unknown) {
  const s = String(e)
  if (s === 'journal-already-undone') return t('journal.err.undone')
  if (s === 'journal-not-found') return t('journal.err.gone')
  if (s === 'journal-unavailable') return t('journal.err.unavailable')
  return S.isNotResponding(e) ? t('pane.notResponding') : s
}

let seq = 0
export async function load(more = false) {
  const mine = ++seq
  jr.loading = true
  try {
    const r = await invoke<{ rows: Row[]; matched: number; all: number; setAside: string | null }>('journal_list', { query: { search: jr.search, kind: jr.kind || null, offset: more ? jr.rows.length : 0, limit: PAGE } })
    if (mine !== seq) return
    jr.rows = more ? [...jr.rows, ...r.rows] : r.rows
    Object.assign(jr, { matched: r.matched, all: r.all, setAside: r.setAside })
  } catch (e) {
    S.fail(explain(e))
  } finally {
    if (mine === seq) jr.loading = false
  }
}

export function openJournal() {
  // A running undo's preview and an unread report stay: closing the dialog must not lose its outcome.
  Object.assign(jr, { open: true, tab: 'list', expanded: null, pairs: [] })
  if (!jr.running) jr.preview = null
  void load()
}

export function closeJournal() {
  // An undo that's running goes on; its report stays for when the dialog opens next.
  jr.open = false
}

export async function expand(id: number) {
  if (jr.expanded === id) return void (jr.expanded = null)
  jr.expanded = id
  jr.pairs = []
  try {
    const pairs = await invoke<PairNow[]>('journal_entry', { id })
    if (jr.expanded === id) jr.pairs = pairs
  } catch (e) {
    S.fail(explain(e))
  }
}

export async function findWhere() {
  const query = jr.whereQuery.trim()
  if (!query) return void (jr.chains = null)
  try {
    jr.chains = await invoke<Chain[]>('journal_where', { query })
  } catch (e) {
    S.fail(explain(e))
  }
}

export async function startUndo(id: number) {
  try {
    jr.named = new Set()
    jr.report = null
    jr.preview = await invoke<Preview>('journal_preview', { id })
  } catch (e) {
    S.fail(explain(e))
  }
}

export async function runUndo() {
  const p = jr.preview
  if (!p || jr.running) return
  if (stackUndoRunning()) return S.toast({ title: t('journal.stackBusy'), tone: 'info' }, 3000)
  jr.running = true
  try {
    const report = await whileJournalUndo(async () => {
      const r = await invoke<Report>('journal_apply', { id: p.id, named: [...jr.named], label: t('history.undone', { label: p.label }), record: settings.journal })
      if (r.applied) await forgetUndoOf(p.id)
      return r
    })
    jr.report = report
    jr.preview = null
    jr.expanded = null
    await load()
    S.refreshAll()
  } catch (e) {
    S.fail(explain(e))
  } finally {
    jr.running = false
  }
}

export function confirmClear() {
  S.state.confirm = {
    title: t('journal.clearTitle'),
    desc: t('journal.clearDesc'),
    danger: true,
    ok: t('journal.clearOk'),
    run: async () => {
      try {
        await invoke('journal_clear')
        S.toast({ title: t('journal.cleared'), tone: 'ok' }, 2500)
        if (jr.open) {
          Object.assign(jr, { expanded: null, chains: null, preview: null, report: null })
          await load()
        }
      } catch (e) {
        S.fail(explain(e))
      }
    },
  }
}
