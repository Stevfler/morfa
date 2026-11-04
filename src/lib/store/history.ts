import { invoke } from '@tauri-apps/api/core'
import { state } from './state'
import { fail, REPORTED, toast } from './toast'
import { refreshAll } from './navigation'
import type { JournalKind, JournalLink, JournalStatus, UndoItem } from './types'
import { t } from '../i18n'
import { settings } from '../settings'

let saveWarned = false

/** Adds a finished operation to the journal. Never throws: losing a record must not fail the operation. */
export function journalRecord(kind: JournalKind, label: string, pairs: [string, string][], status: JournalStatus = 'done', error?: unknown, started = Date.now()): Promise<number | null> {
  if (!settings.journal) return Promise.resolve(null)
  return invoke<number>('journal_record', { input: { kind, label, status, error: error === undefined ? null : String(error), pairs, started } }).catch((e) => {
    console.error('journal', e)
    if (!saveWarned) toast({ title: t('journal.saveFailed'), desc: String(e) === 'journal-unavailable' ? t('journal.err.unavailable') : String(e), tone: 'error' }, 8000)
    saveWarned = true
    return null
  })
}

/** Records the operation and gives the undo item its link. */
export function journalLink(kind: JournalKind, label: string, pairs: () => [string, string][], status: JournalStatus = 'done', error?: unknown, started = Date.now()): JournalLink {
  return { id: journalRecord(kind, label, pairs(), status, error, started), kind, pairs }
}

/** A record stuck on a dead mount mustn't hold up undo; the journal's own checks still protect it. */
const linkedId = (j: JournalLink) => Promise.race([j.id, new Promise<null>((done) => setTimeout(() => done(null), 3000))])

/** Ctrl+Z calls running: a journal undo waits for them, and they wait for it. */
let stackUndos = 0
let journalUndos = 0
export const stackUndoRunning = () => stackUndos > 0

/** Runs a journal undo with Ctrl+Z held off. */
export async function whileJournalUndo<T>(run: () => Promise<T>): Promise<T> {
  journalUndos++
  try {
    return await run()
  } finally {
    journalUndos--
  }
}

/** The journal undid this entry, so Ctrl+Z and redo mustn't. */
export async function forgetUndoOf(id: number) {
  const drop = async (list: UndoItem[]) => {
    const ids = await Promise.all(list.map((i) => (i.journal ? linkedId(i.journal) : null)))
    return ids.findIndex((x) => x === id)
  }
  for (const list of [state.undo, state.redo]) {
    const at = await drop(list)
    if (at >= 0) list.splice(at, 1)
  }
}

export function pushUndo(item: UndoItem, desc?: string) {
  state.undo.push(item)
  state.redo = []
  if (state.undo.length > 100) state.undo.shift()
  toast({ title: item.label, desc, tone: 'ok', action: { label: t('cmd.undo'), run: undo } })
}

export async function undo() {
  if (journalUndos) return toast({ title: t('journal.busy'), tone: 'info' }, 2000)
  const item = state.undo.pop()
  if (!item) return toast({ title: t('history.nothingUndo'), tone: 'info' }, 2000)
  stackUndos++
  try {
    await item.undo()
    if (item.journal) {
      const id = await linkedId(item.journal)
      // Gone from the journal (cleared, or too old) is fine; it just has nothing to mark.
      if (id !== null) await invoke('journal_mark_undone', { id, label: t('history.undone', { label: item.label }), record: settings.journal }).catch((e) => console.error('journal', e))
    }
    state.redo.push(item)
    toast({ title: t('history.undone', { label: item.label }), tone: 'info', action: { label: t('cmd.redo'), run: redo } })
  } catch (e) {
    // macOS can't restore from Trash programmatically; point the user at Finder's "Put Back".
    if (String(e).includes('UNSUPPORTED_RESTORE:'))
      toast(
        { title: t('history.noRestore'), desc: t('history.noRestoreDesc'), tone: 'info', action: { label: t('history.showTrash'), run: () => invoke('reveal_trash').catch(fail) } },
        10000,
      )
    else {
      if (item.retry) {
        state.undo.push(item)
        // It may keep failing: older items must stay reachable.
        const action = { label: t('history.removeUndo'), run: () => void state.undo.splice(state.undo.indexOf(item) >>> 0, 1) }
        if (e === REPORTED) toast({ title: t('history.undoKept', { label: item.label }), tone: 'info', action }, 15000)
        else fail(e, action)
      } else fail(e)
    }
  } finally {
    stackUndos--
  }
  refreshAll()
}

export async function redo() {
  if (journalUndos) return toast({ title: t('journal.busy'), tone: 'info' }, 2000)
  const item = state.redo.pop()
  if (!item) return toast({ title: t('history.nothingRedo'), tone: 'info' }, 2000)
  try {
    const started = Date.now()
    await item.redo()
    // Done again: a new operation, with an entry of its own. The undone one stays as it was.
    if (item.journal) item.journal.id = journalRecord(item.journal.kind, item.label, item.journal.pairs(), 'done', undefined, started)
    state.undo.push(item)
    toast({ title: t('history.redone', { label: item.label }), tone: 'info' })
  } catch (e) {
    if (item.retry) state.redo.push(item)
    fail(e)
  }
  refreshAll()
}
