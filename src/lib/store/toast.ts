import { nextId, state } from './state'
import type { Toast } from './types'
import { t } from '../i18n'

export function toast(t: Omit<Toast, 'id'>, ms = 5000) {
  const id = nextId()
  state.toasts.push({ id, ...t })
  setTimeout(() => dismiss(id), ms)
  return id
}
export const dismiss = (id: number) => (state.toasts = state.toasts.filter((t) => t.id !== id))

export function fail(e: unknown) {
  toast({ title: t('error.generic'), desc: String(e), tone: 'error' }, 8000)
}
