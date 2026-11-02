import { invoke } from '@tauri-apps/api/core'
import { basename, join, keyLabel, parent } from '../util'
import { nextId, pane, state } from './state'
import { targets } from './view'
import { fail, toast } from './toast'
import { pushUndo } from './history'
import { load_, refreshAll } from './navigation'
import type { Pane } from './types'

const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`
const describe = (paths: string[]) => (paths.length === 1 ? `“${basename(paths[0])}”` : plural(paths.length, 'item'))

export function copyToClipboard(mode: 'copy' | 'move', p: Pane = pane()) {
  const t = targets(p)
  if (!t.length) return
  state.clipboard = { mode, paths: t.map((e) => e.path) }
  toast(
    { title: mode === 'copy' ? `Copied ${describe(state.clipboard.paths)}` : `Cut ${describe(state.clipboard.paths)}`, desc: `Press ${keyLabel('Ctrl+V')} in any folder to paste`, tone: 'info' },
    2500,
  )
}

export async function paste(p: Pane = pane()) {
  const c = state.clipboard
  if (!c) return toast({ title: 'Clipboard is empty', desc: `Copy (${keyLabel('Ctrl+C')}) or cut (${keyLabel('Ctrl+X')}) something first`, tone: 'info' }, 2500)
  await transfer(c.paths, p.path, c.mode)
  if (c.mode === 'move') state.clipboard = null
}

export async function transfer(sources: string[], dest: string, mode: 'copy' | 'move') {
  if (!sources.length) return
  const id = nextId()
  const verb = mode === 'copy' ? 'Copying' : 'Moving'
  state.ops.push({ id, label: `${verb} ${describe(sources)} → ${basename(dest)}`, done: 0, total: 0, current: '' })
  try {
    const { pairs, error } = await invoke<{ pairs: [string, string][]; error: string | null }>('transfer', { id, sources, dest, mode })
    if (pairs.length) recordTransfer(pairs, dest, mode, error ? `${verb} stopped after ${plural(pairs.length, 'item')}` : undefined)
    if (error) error === 'Cancelled' ? toast({ title: `${verb} cancelled`, tone: 'info' }) : fail(error)
  } catch (e) {
    fail(e)
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

/** Undo entry for a finished (or partly finished) transfer. Redo of a copy re-runs the copy. */
function recordTransfer(pairs: [string, string][], dest: string, mode: 'copy' | 'move', note?: string) {
  const label = `${mode === 'copy' ? 'Copied' : 'Moved'} ${describe(pairs.map(([s]) => s))} to ${basename(dest)}`
  let copies = pairs.map(([, d]) => d)
  const recopy = async () => {
    const r = await invoke<{ pairs: [string, string][]; error: string | null }>('transfer', { id: nextId(), sources: pairs.map(([s]) => s), dest, mode: 'copy' })
    copies = r.pairs.map(([, d]) => d)
    if (r.error) throw r.error
  }
  pushUndo(
    mode === 'copy'
      ? { label, undo: () => invoke('trash_items', { paths: copies }), redo: recopy }
      : {
          label,
          undo: () => invoke('move_pairs', { pairs: pairs.map(([s, d]) => [d, s]) }),
          redo: () => invoke('move_pairs', { pairs }),
        },
    note ?? (pairs.some(([s, d]) => basename(s) !== basename(d)) ? 'Name conflicts were kept as copies — nothing was overwritten' : undefined),
  )
}

export const cancelOp = (id: number) => invoke('cancel_op', { id })

export async function trash(paths = targets().map((e) => e.path)) {
  if (!paths.length) return
  try {
    await invoke('trash_items', { paths })
    pushUndo({
      label: `Moved ${describe(paths)} to Trash`,
      undo: () => invoke('restore_trashed', { paths }),
      redo: () => invoke('trash_items', { paths }),
    })
  } catch (e) {
    fail(e)
  }
  refreshAll()
}

export function deleteForever(paths = targets().map((e) => e.path)) {
  if (!paths.length) return
  state.confirm = {
    title: `Delete ${describe(paths)} forever?`,
    desc: 'This skips the Trash and cannot be undone. Use Delete (without Shift) to move to Trash instead.',
    danger: true,
    ok: 'Delete forever',
    run: async () => {
      try {
        await invoke('delete_items', { paths })
        toast({ title: `Deleted ${describe(paths)}`, tone: 'ok' })
      } catch (e) {
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
  if (/[\\/]/.test(newName)) return fail('A name can’t contain “/” or “\\”')
  const to = join(parent(path)!, newName)
  const prev = p.focusAfterLoad
  p.focusAfterLoad = to // before movePairs' refresh loads the folder
  if (!(await movePairs([[path, to]], `Renamed to “${newName}”`))) p.focusAfterLoad = prev
}

/** Resolves to true on success. */
export async function movePairs(pairs: [string, string][], label: string): Promise<boolean> {
  let ok = false
  try {
    await invoke('move_pairs', { pairs })
    pushUndo({
      label,
      undo: () => invoke('move_pairs', { pairs: pairs.map(([a, b]) => [b, a]).reverse() }),
      redo: () => invoke('move_pairs', { pairs }),
    })
    ok = true
  } catch (e) {
    fail(e)
  }
  refreshAll()
  return ok
}

export function startRename(p: Pane = pane()) {
  const t = targets(p)
  if (t.length > 1) state.batchRename = t.map((e) => e.path)
  else if (t.length === 1) state.renaming = t[0].path
}

export async function create(kind: 'dir' | 'file', p: Pane = pane()) {
  try {
    const path = await invoke<string>(kind === 'dir' ? 'create_dir' : 'create_file', {
      parent: p.path,
      name: kind === 'dir' ? 'New folder' : 'Untitled.txt',
    })
    p.filter = ''
    p.focusAfterLoad = path
    let current = path
    const name = basename(path)
    pushUndo({
      label: `Created “${name}”`,
      undo: () => invoke('trash_items', { paths: [current] }),
      redo: async () => {
        current = await invoke<string>(kind === 'dir' ? 'create_dir' : 'create_file', { parent: parent(current)!, name: basename(current) })
      },
    })
    await load_(p)
    state.renaming = path // name it right away
  } catch (e) {
    fail(e)
  }
}
