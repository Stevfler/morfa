import { reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { state, pane, go, toast } from '../../lib/store'
import type { Drive, Place } from '../../lib/store/types'
import { t } from '../../lib/i18n'

export const drives = reactive({
  list: [] as Drive[],
  /** Device or mount path currently being mounted/unmounted. */
  busy: new Set<string>(),
  /** Encrypted drive waiting for a passphrase. */
  unlock: null as Drive | null,
})

const USER_MOUNTS = ['/run/media/', '/media/', '/mnt/', '/Volumes/']
export const ejectable = (p: Place) => p.kind === 'removable' || USER_MOUNTS.some((r) => p.path.startsWith(r))

let pending: Promise<void> | null = null
export function refreshDrives() {
  pending ??= Promise.all([
    invoke<Place[]>('places').then((v) => (state.places = v)),
    invoke<Drive[]>('list_drives').then((v) => (drives.list = v)),
  ]).then(() => {}, () => {}).finally(() => (pending = null))
  return pending
}

/** Mount (passphrase for encrypted drives), then open it. Throws the friendly error message. */
export async function mountDrive(d: Drive, passphrase?: string) {
  drives.busy.add(d.device)
  try {
    const path = await invoke<string>('mount_drive', { device: d.device, passphrase: passphrase ?? null })
    await refreshDrives()
    go(pane(), path)
  } finally {
    drives.busy.delete(d.device)
  }
}

export function clickDrive(d: Drive) {
  if (drives.busy.has(d.device)) return
  if (d.encrypted) drives.unlock = d
  else mountDrive(d).catch((e) => toast({ title: t('drives.mountFail', { name: d.name || t('drives.drive') }), desc: String(e), tone: 'error' }))
}

export async function unmountPlace(p: Place) {
  const eject = p.kind === 'removable'
  // Move panes off the drive first so nothing in Morfa keeps it busy.
  const moved = state.tabs.flatMap((t) => t.panes)
    .filter((pn) => pn.path === p.path || pn.path.startsWith(p.path.replace(/\/?$/, '/')))
    .map((pn) => ({ pn, from: pn.path }))
  for (const { pn } of moved) go(pn, state.home)
  drives.busy.add(p.path)
  try {
    await invoke('unmount_drive', { path: p.path, eject })
    toast({ title: eject ? `${p.name} can be safely removed` : `${p.name} unmounted`, tone: 'ok' }, 3000)
  } catch (e) {
    for (const { pn, from } of moved) go(pn, from, { history: false })
    toast({ title: t('drives.unmountFail', { name: p.name }), desc: String(e), tone: 'error' })
  } finally {
    drives.busy.delete(p.path)
    refreshDrives()
  }
}
