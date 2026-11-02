import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { nextId, state, toast } from '../store'
import { resetConvertCaches } from '../convert/caps'
import { t } from '../i18n'
import { fmtSize as fmt } from '../util'

export type OfficeStatus = {
  installed: boolean
  systemOffice: boolean
  path: string | null
  sizeOnDisk: number
  downloadSize: number
  version: string
  supported: boolean
}
type Phase = 'download' | 'extract' | 'prune' | 'done'
type Progress = { phase: Phase; done: number; total: number }

export const office = ref<OfficeStatus | null>(null)
/** Op id of the running install, or null. */
export const installing = ref<number | null>(null)
export const progress = ref<Progress | null>(null)
/** Bumped after install/remove so open views re-ask for convert targets. */
export const officeEpoch = ref(0)

let pending: Promise<OfficeStatus | null> | null = null
/** Module status; cached unless forced. Never throws (null on failure). */
export function refreshOffice(force = false): Promise<OfficeStatus | null> {
  if (force || !pending)
    pending = invoke<OfficeStatus>('office_status').then(
      (s) => (office.value = s),
      () => ((pending = null), null),
    )
  return pending
}

/** True when a missing office engine can be fixed by downloading the module. */
export const canDownload = () => !!office.value?.supported && !office.value.installed && !office.value.systemOffice

const phaseText = (p: Phase) => t(`office.phase.${p}`)

export async function installOffice(): Promise<void> {
  if (installing.value != null) return
  const id = nextId()
  installing.value = id
  const total = office.value?.downloadSize ?? 0
  progress.value = { phase: 'download', done: 0, total }
  state.ops.push({ id, label: t('office.downloading'), done: 0, total, current: phaseText('download'), unit: 'bytes', kind: 'office' })
  const un = await listen<Progress>('office-progress', ({ payload }) => {
    progress.value = payload
    const op = state.ops.find((o) => o.id === id)
    if (!op) return
    op.label = payload.phase === 'download' ? t('office.downloading') : t('office.installing')
    op.current = phaseText(payload.phase)
    op.done = payload.done
    op.total = payload.total
  })
  try {
    await invoke('office_install')
    toast({ title: t('office.installed'), desc: t('office.installedDesc'), tone: 'ok' })
  } catch (e) {
    const msg = String(e)
    if (/cancel/i.test(msg)) toast({ title: t('office.cancelled'), tone: 'info' })
    else toast({ title: t('office.installFailed'), desc: msg, tone: 'error' }, 10000)
  } finally {
    un()
    state.ops = state.ops.filter((o) => o.id !== id)
    installing.value = null
    progress.value = null
    await changed()
  }
}

export const cancelOffice = () => invoke('office_cancel')

export function removeOffice() {
  const size = office.value?.sizeOnDisk ?? 0
  state.confirm = {
    title: t('office.removeTitle'),
    desc: t('office.removeDesc'),
    danger: true,
    ok: t('office.remove'),
    run: async () => {
      try {
        await invoke('office_remove')
        toast({ title: t('office.removed'), desc: size ? t('office.freed', { size: fmt(size) }) : undefined, tone: 'ok' })
      } catch (e) {
        toast({ title: t('office.removeFailed'), desc: String(e), tone: 'error' }, 8000)
      } finally {
        await changed()
      }
    },
  }
}

async function changed() {
  resetConvertCaches()
  officeEpoch.value++
  await refreshOffice(true)
}
