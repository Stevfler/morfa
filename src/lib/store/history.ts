import { invoke } from '@tauri-apps/api/core'
import { state } from './state'
import { fail, toast } from './toast'
import { refreshAll } from './navigation'
import type { UndoItem } from './types'
import { t } from '../i18n'

export function pushUndo(item: UndoItem, desc?: string) {
  state.undo.push(item)
  state.redo = []
  if (state.undo.length > 100) state.undo.shift()
  toast({ title: item.label, desc, tone: 'ok', action: { label: t('cmd.undo'), run: undo } })
}

export async function undo() {
  const item = state.undo.pop()
  if (!item) return toast({ title: t('history.nothingUndo'), tone: 'info' }, 2000)
  try {
    await item.undo()
    state.redo.push(item)
    toast({ title: t('history.undone', { label: item.label }), tone: 'info', action: { label: t('cmd.redo'), run: redo } })
  } catch (e) {
    // macOS can't restore from Trash programmatically; point the user at Finder's "Put Back".
    if (String(e).includes('UNSUPPORTED_RESTORE:'))
      toast(
        { title: t('history.noRestore'), desc: t('history.noRestoreDesc'), tone: 'info', action: { label: t('history.showTrash'), run: () => invoke('reveal_trash').catch(fail) } },
        10000,
      )
    else fail(e)
  }
  refreshAll()
}

export async function redo() {
  const item = state.redo.pop()
  if (!item) return toast({ title: t('history.nothingRedo'), tone: 'info' }, 2000)
  try {
    await item.redo()
    state.undo.push(item)
    toast({ title: t('history.redone', { label: item.label }), tone: 'info' })
  } catch (e) {
    fail(e)
  }
  refreshAll()
}
