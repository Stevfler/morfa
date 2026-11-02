import { invoke } from '@tauri-apps/api/core'
import { state } from './state'
import { fail, toast } from './toast'
import { refreshAll } from './navigation'
import type { UndoItem } from './types'

export function pushUndo(item: UndoItem, desc?: string) {
  state.undo.push(item)
  state.redo = []
  if (state.undo.length > 100) state.undo.shift()
  toast({ title: item.label, desc, tone: 'ok', action: { label: 'Undo', run: undo } })
}

export async function undo() {
  const item = state.undo.pop()
  if (!item) return toast({ title: 'Nothing to undo', tone: 'info' }, 2000)
  try {
    await item.undo()
    state.redo.push(item)
    toast({ title: `Undone: ${item.label}`, tone: 'info', action: { label: 'Redo', run: redo } })
  } catch (e) {
    // macOS can't restore from Trash programmatically; point the user at Finder's "Put Back".
    if (String(e).includes('UNSUPPORTED_RESTORE:'))
      toast(
        { title: "Restore isn't available on macOS", desc: 'Open the Trash and use “Put Back” to restore the items.', tone: 'info', action: { label: 'Show Trash', run: () => invoke('reveal_trash').catch(fail) } },
        10000,
      )
    else fail(e)
  }
  refreshAll()
}

export async function redo() {
  const item = state.redo.pop()
  if (!item) return toast({ title: 'Nothing to redo', tone: 'info' }, 2000)
  try {
    await item.redo()
    state.undo.push(item)
    toast({ title: `Redone: ${item.label}`, tone: 'info' })
  } catch (e) {
    fail(e)
  }
  refreshAll()
}
