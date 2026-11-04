import { nextId, state } from './state'
import { isNotResponding, parseHostKeyChange, type Toast } from './types'
import { t } from '../i18n'

export function toast(t: Omit<Toast, 'id'>, ms = 5000) {
  const id = nextId()
  state.toasts.push({ id, ...t })
  setTimeout(() => dismiss(id), ms)
  return id
}
export const dismiss = (id: number) => (state.toasts = state.toasts.filter((t) => t.id !== id))

/** Thrown when the user has already been told; callers only need to know it failed. */
export const REPORTED = Symbol('reported')

export function fail(e: unknown, action?: Toast['action']) {
  if (e === REPORTED) return
  const changed = parseHostKeyChange(e)
  if (changed) return void toast({ title: t('cloud.hostKeyChangedTitle'), desc: t('cloud.hostKeyChanged', changed), tone: 'error' }, 15000)
  toast({ title: isNotResponding(e) ? t('pane.notResponding') : t('error.generic'), desc: isNotResponding(e) ? t('pane.notRespondingHint') : String(e), tone: 'error', action }, 8000)
}
