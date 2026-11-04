import { invoke } from '@tauri-apps/api/core'
import { openPath } from '@tauri-apps/plugin-opener'
import { basename, join, keyLabel, parent } from '../util'
import { nextId, otherPane, pane, state } from './state'
import { targets } from './view'
import { fail, REPORTED, toast } from './toast'
import { resolveStranded } from './moves'
import { journalLink, journalRecord, pushUndo } from './history'
import { load_, open, refreshAll } from './navigation'
import { isSpecial, parseArchive, type JournalKind, type Pane } from './types'
import { extractEntries } from './archive'
import { t } from '../i18n'

const items = (count: number) => t('core.items', { count })
const describe = (paths: string[]) => (paths.length === 1 ? t('core.quoted', { name: basename(paths[0]) }) : items(paths.length))

export function copyToClipboard(mode: 'copy' | 'move', p: Pane = pane()) {
  const sel = targets(p)
  if (!sel.length) return
  if (parseArchive(p.path)) return toast({ title: t('archive.readOnly'), desc: t('archive.readOnlyHint'), tone: 'info' }, 3000)
  state.clipboard = { mode, paths: sel.map((e) => e.path) }
  toast(
    { title: t(mode === 'copy' ? 'fileops.copied' : 'fileops.cut', { what: describe(state.clipboard.paths) }), desc: t('fileops.pasteHint', { key: keyLabel('Ctrl+V') }), tone: 'info' },
    2500,
  )
}

export async function paste(p: Pane = pane()) {
  const c = state.clipboard
  if (!c) return toast({ title: t('fileops.clipEmpty'), desc: t('fileops.clipEmptyHint', { copy: keyLabel('Ctrl+C'), cut: keyLabel('Ctrl+X') }), tone: 'info' }, 2500)
  await transfer(c.paths, p.path, c.mode)
  if (c.mode === 'move') state.clipboard = null
}

export async function transfer(sources: string[], dest: string, mode: 'copy' | 'move') {
  // Pseudo-locations (e.g. the Drives view) are not folders: never send them to the backend.
  if (!sources.length || isSpecial(dest) || sources.some(isSpecial)) return
  const id = nextId()
  const started = Date.now()
  const cp = mode === 'copy'
  state.ops.push({ id, label: t(cp ? 'fileops.copying' : 'fileops.moving', { what: describe(sources), dest: basename(dest) }), done: 0, total: 0, current: '' })
  try {
    const { pairs, error } = await invoke<{ pairs: [string, string][]; error: string | null }>('transfer', { id, sources, dest, mode })
    const status = !error ? 'done' : error === 'Cancelled' ? 'cancelled' : pairs.length ? 'partial' : 'failed'
    if (pairs.length) recordTransfer(pairs, dest, mode, error ? t(cp ? 'fileops.copyStopped' : 'fileops.moveStopped', { what: items(pairs.length) }) : undefined, status, error, started)
    else if (error) journalRecord(mode, transferLabel(sources, dest, mode), sources.map((s) => [s, '']), status, error, started)
    if (error) error === 'Cancelled' ? toast({ title: t(cp ? 'fileops.copyCancelled' : 'fileops.moveCancelled'), tone: 'info' }) : fail(error)
  } catch (e) {
    journalRecord(mode, transferLabel(sources, dest, mode), sources.map((s) => [s, '']), 'failed', e, started)
    fail(e)
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

const transferLabel = (sources: string[], dest: string, mode: 'copy' | 'move') => t(mode === 'copy' ? 'fileops.copiedTo' : 'fileops.movedTo', { what: describe(sources), dest: basename(dest) })

/** Undo entry for a finished (or partly finished) transfer. Redo of a copy re-runs the copy. */
function recordTransfer(pairs: [string, string][], dest: string, mode: 'copy' | 'move', note: string | undefined, status: 'done' | 'partial' | 'cancelled' | 'failed', error: string | null, started: number) {
  const label = transferLabel(pairs.map(([s]) => s), dest, mode)
  let copies = pairs.map(([, d]) => d)
  const sources = pairs.map(([s]) => s)
  const recopy = async () => {
    const r = await invoke<{ pairs: [string, string][]; error: string | null }>('transfer', { id: nextId(), sources: pairs.map(([s]) => s), dest, mode: 'copy' })
    copies = r.pairs.map(([, d]) => d)
    if (r.error) throw r.error
  }
  pushUndo(
    mode === 'copy'
      ? { label, undo: () => invoke('trash_items', { paths: copies }), redo: recopy, journal: journalLink('copy', label, () => copies.map((d, i) => [sources[i] ?? '', d]), status, error ?? undefined, started) }
      : {
          label,
          retry: true,
          undo: () => replayMoves(pairs.map(([s, d]): [string, string] => [d, s]), 'move', label, true),
          redo: () => replayMoves(pairs, 'move', label),
          journal: journalLink('move', label, () => pairs, status, error ?? undefined, started),
        },
    note ?? (pairs.some(([s, d]) => basename(s) !== basename(d)) ? t('fileops.conflicts') : undefined),
  )
}

export function cancelOp(id: number) {
  const kind = state.ops.find((o) => o.id === id)?.kind
  if (kind === 'autoconvert') return import('../convert/auto').then((m) => m.cancelAutoOp(id))
  if (kind === 'office') return invoke('office_cancel')
  if (kind === 'archive') return invoke('archive_cancel', { id })
  if (kind === 'compare') return invoke('compare_cancel', { id })
  if (kind === 'dedupe') return invoke('dedupe_cancel', { id })
  return invoke(kind === 'convert' ? 'convert_cancel' : 'cancel_op', { id })
}

export async function trash(paths = targets().map((e) => e.path)) {
  if (!paths.length) return
  if (paths.some(parseArchive)) return toast({ title: t('archive.readOnly'), tone: 'info' }, 2500)
  const started = Date.now()
  try {
    await invoke('trash_items', { paths })
    const label = t('fileops.trashed', { what: describe(paths) })
    pushUndo({
      label,
      undo: () => invoke('restore_trashed', { paths }),
      redo: () => invoke('trash_items', { paths }),
      journal: journalLink('trash', label, () => paths.map((p) => [p, '']), 'done', undefined, started),
    })
  } catch (e) {
    journalRecord('trash', t('fileops.trashed', { what: describe(paths) }), paths.map((p) => [p, '']), 'failed', e, started)
    fail(e)
  }
  refreshAll()
}

export function deleteForever(paths = targets().map((e) => e.path)) {
  if (!paths.length) return
  if (paths.some(parseArchive)) return toast({ title: t('archive.readOnly'), tone: 'info' }, 2500)
  state.confirm = {
    title: t('fileops.deleteTitle', { what: describe(paths) }),
    desc: t('fileops.deleteDesc'),
    danger: true,
    ok: t('cmd.delete'),
    run: async () => {
      const started = Date.now()
      try {
        await invoke('delete_items', { paths })
        toast({ title: t('fileops.deleted', { what: describe(paths) }), tone: 'ok' })
        journalRecord('delete', t('fileops.deleted', { what: describe(paths) }), paths.map((p) => [p, '']), 'done', undefined, started)
      } catch (e) {
        journalRecord('delete', t('fileops.deleted', { what: describe(paths) }), paths.map((p) => [p, '']), 'failed', e, started)
        fail(e)
      }
      refreshAll()
    },
  }
}

export async function renameTo(path: string, newName: string, p: Pane = pane()) {
  // Enter/Escape unmount the input, which may fire blur: commit only once.
  if (state.renaming !== path) return
  state.renaming = null
  newName = newName.trim()
  if (!newName || newName === basename(path)) return
  if (/[\\/]/.test(newName)) return fail(t('fileops.badName'))
  const to = join(parent(path)!, newName)
  const prev = p.focusAfterLoad
  p.focusAfterLoad = to // before movePairs' refresh loads the folder
  if (!(await movePairs([[path, to]], t('fileops.renamed', { name: newName })))) p.focusAfterLoad = prev
}

/** A rollback failed. Each file left behind is listed with the name it belongs under, also in the
 *  journal, where undoing the entry moves it back there. A name that can't be told is said to be unknown. */
function reportStranded(kind: JournalKind, label: string, steps: [string, string][], stranded: [string, string][], error: string, started: number, isUndo = false) {
  const { named, unknown } = resolveStranded(steps, stranded, isUndo)
  if (named.length) journalRecord(kind, label, named, 'partial', error, started)
  else journalRecord(kind, label, unknown.map((at): [string, string] => [at, '']), 'failed', error, started)
  const lines = [...named.map(([name, at]) => `${basename(at)} → ${basename(name)}`), ...unknown.map((at) => t('fileops.strandedUnknown', { name: basename(at) }))]
  const shown = lines.slice(0, 6)
  if (lines.length > 6) shown.push(`+${lines.length - 6}`)
  toast({ title: t('fileops.stranded', { count: stranded.length }), desc: `${error}\n${shown.join('\n')}`, tone: 'error' }, 60000)
}

/** Undo or redo of moves: all or nothing, and what stands is reported. Throws so the item is kept. */
async function replayMoves(steps: [string, string][], kind: JournalKind, label: string, isUndo = false) {
  const started = Date.now()
  const r = await invoke<{ completed: [string, string][]; error: string | null }>('move_pairs_report', { pairs: steps })
  if (!r.error) return
  if (!r.completed.length) throw t('fileops.nothingChanged', { error: r.error })
  reportStranded(kind, label, steps, r.completed, r.error, started, isUndo)
  throw REPORTED
}

/** Resolves to true on success. `record`: the moves as the user means them, when `pairs` takes detours. */
export async function movePairs(pairs: [string, string][], label: string, kind: JournalKind = 'rename', record: [string, string][] = pairs): Promise<boolean> {
  let ok = false
  const started = Date.now()
  // The call that failed says what stands; nothing is ever guessed from the disk.
  const asked = record.map(([from]): [string, string] => [from, ''])
  try {
    const r = await invoke<{ completed: [string, string][]; error: string | null }>('move_pairs_report', { pairs })
    if (r.error) {
      if (r.completed.length) reportStranded(kind, label, pairs, r.completed, r.error, started)
      else {
        journalRecord(kind, label, asked, 'failed', r.error, started)
        fail(t('fileops.nothingChanged', { error: r.error }))
      }
      refreshAll()
      return false
    }
    pushUndo({
      label,
      retry: true,
      undo: () => replayMoves(pairs.map(([a, b]): [string, string] => [b, a]).reverse(), kind, label, true),
      redo: () => replayMoves(pairs, kind, label),
      journal: journalLink(kind, label, () => record, 'done', undefined, started),
    })
    ok = true
  } catch (e) {
    journalRecord(kind, label, asked, 'failed', e, started)
    fail(e)
  }
  refreshAll()
  return ok
}

/** Batch rename needs real folders: not offered on pseudo-paths (Trash, cloud, Drives…). */
export const canBatchRename = (p: Pane = pane()) => !isSpecial(p.path)

export function startRename(p: Pane = pane()) {
  const sel = targets(p)
  if (parseArchive(p.path)) return toast({ title: t('archive.readOnly'), tone: 'info' }, 2500)
  if (sel.length > 1) {
    if (canBatchRename(p)) state.batchRename = sel.map((e) => e.path)
  }
  else if (sel.length === 1) state.renaming = sel[0].path
}

export async function create(kind: 'dir' | 'file', p: Pane = pane()) {
  const started = Date.now()
  try {
    const path = await invoke<string>(kind === 'dir' ? 'create_dir' : 'create_file', {
      parent: p.path,
      name: t(kind === 'dir' ? 'fileops.newFolder' : 'fileops.newFile'),
    })
    p.filter = ''
    p.focusAfterLoad = path
    let current = path
    const name = basename(path)
    pushUndo({
      label: t('fileops.created', { name }),
      undo: () => invoke('trash_items', { paths: [current] }),
      redo: async () => {
        current = await invoke<string>(kind === 'dir' ? 'create_dir' : 'create_file', { parent: parent(current)!, name: basename(current) })
      },
      journal: journalLink('create', t('fileops.created', { name }), () => [['', current]], 'done', undefined, started),
    })
    await load_(p)
    state.renaming = path // name it right away
  } catch (e) {
    fail(e)
  }
}

/** Why copy/move to the other pane can't run right now (null = it can). */
export function toOtherBlocked(p: Pane = pane()): string | null {
  const o = otherPane()
  if (!o) return t('fileops.noDual')
  // Copying out of an archive unpacks; only the destination has to be a real folder.
  if (parseArchive(p.path) && !isSpecial(o.path)) return targets(p).length ? null : t('fileops.nothingSelected')
  if (isSpecial(o.path) || isSpecial(p.path)) return t('fileops.bothFolders')
  if (o.path === p.path) return t('fileops.sameFolder')
  return targets(p).length ? null : t('fileops.nothingSelected')
}

/** Commander-style F5/F6: copy or move the selection into the other pane's folder (toast + undo, no dialog). */
export function toOther(mode: 'copy' | 'move') {
  const why = toOtherBlocked()
  if (why) return toast({ title: why, tone: 'info' })
  if (parseArchive(pane().path)) {
    if (mode === 'move') return toast({ title: t('archive.readOnly'), desc: t('archive.readOnlyHint'), tone: 'info' }, 3000)
    return extractEntries(pane(), targets(), otherPane()!.path)
  }
  transfer(targets().map((e) => e.path), otherPane()!.path, mode)
}

/** F4: open the item under the cursor with the system's default app (folders too). */
export function openDefault(p: Pane = pane()) {
  const e = targets(p)[0]
  if (e && parseArchive(e.path)) return void open(p, e)
  if (e) openPath(e.path).catch(fail)
}
