import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { state, pane, go, load_, toast, fail, cloudPath, parseCloud } from '../store'
import type { CloudAccount, Entry } from '../store'
import type { CloudStatus } from '../store/types'
import { within } from '../store/navigation'
import { t } from '../i18n'

export const refreshCloud = () =>
  invoke<CloudAccount[]>('cloud_accounts').then((v) => (state.cloud = v), () => {})

/** Reload panes showing this account's remote folders or its local mirror. */
function reloadPanes(id: string) {
  const a = state.cloud.find((x) => x.id === id)
  for (const tb of state.tabs)
    for (const p of tb.panes)
      if (!p.search && (parseCloud(p.path)?.id === id || (a && within(p.path, a.localRoot)))) load_(p)
}

export async function initCloud() {
  await refreshCloud()
  await listen<CloudStatus & { id: string }>('cloud-status', ({ payload }) => {
    const a = state.cloud.find((x) => x.id === payload.id)
    if (!a) return
    const wasSyncing = a.status.state === 'syncing'
    a.status = { state: payload.state, error: payload.error, current: payload.current, last: payload.last }
    // Marks change from "pending" to "synced" when a run finishes.
    if (wasSyncing && payload.state !== 'syncing') reloadPanes(a.id)
  })
  await listen<string>('cloud-changed', ({ payload }) => reloadPanes(payload))
  // Coming back to the window is a good moment to pick up remote changes.
  window.addEventListener('focus', () => invoke('cloud_sync', { id: null }).catch(() => {}))
}

export const accountOf = (id: string) => state.cloud.find((a) => a.id === id)

/** Account and rel path for a cloud entry or a local path inside a mirror folder. */
export async function cloudTarget(path: string): Promise<{ id: string; rel: string } | null> {
  const c = parseCloud(path)
  if (c) return c
  if (!state.cloud.some((a) => within(path, a.localRoot))) return null
  const r = await invoke<[string, string] | null>('cloud_resolve', { path })
  return r ? { id: r[0], rel: r[1] } : null
}

async function group(entries: Entry[]) {
  const by = new Map<string, string[]>()
  for (const e of entries) {
    const c = await cloudTarget(e.path)
    if (c) by.set(c.id, [...(by.get(c.id) ?? []), c.rel])
  }
  return by
}

export async function pin(entries: Entry[]) {
  try {
    for (const [id, rels] of await group(entries)) {
      await invoke('cloud_pin', { id, rels })
      const a = accountOf(id)
      if (a) a.pins = [...new Set([...a.pins, ...rels])]
    }
    for (const e of entries) e.cloud = 'pending'
    toast({ title: t('cloud.pinned', { count: entries.length }), tone: 'ok' }, 3000)
  } catch (e) {
    fail(e)
  }
}

export function unpin(entries: Entry[], deleteLocal: boolean) {
  const run = async () => {
    try {
      for (const [id, rels] of await group(entries)) {
        await invoke('cloud_unpin', { id, rels, deleteLocal })
        await refreshCloud()
        reloadPanes(id)
      }
    } catch (e) {
      fail(e)
    }
  }
  if (!deleteLocal) return run()
  state.confirm = {
    title: t('cloud.unpinDeleteTitle', { count: entries.length }),
    desc: t('cloud.unpinDeleteDesc'),
    danger: true,
    ok: t('cloud.unpinDeleteOk'),
    run,
  }
}

export async function showLocal(e: Entry) {
  const local = e.local
  if (!local) return
  const dir = e.isDir ? local : local.replace(/[\\/][^\\/]*$/, '')
  try {
    await invoke('stat', { path: dir })
    go(pane(), dir, { focus: e.isDir ? undefined : local })
  } catch {
    toast({ title: t('cloud.notOnDevice'), tone: 'info' }, 3000)
  }
}

/** Jump from a mirrored local item to the same place in the cloud view. */
export async function showInCloud(path: string) {
  const c = await cloudTarget(path)
  if (!c) return
  const parentRel = c.rel.split('/').slice(0, -1).join('/')
  go(pane(), cloudPath(c.id, parentRel), { focus: cloudPath(c.id, c.rel) })
}

export function openWeb(id: string, rel = '') {
  const a = accountOf(id)
  if (a) openUrl(`${a.server}/apps/files/?dir=/${encodeURI(rel)}`).catch(fail)
}

export const syncNow = (id: string) => invoke('cloud_sync', { id }).catch(fail)

export function removeAccount(a: CloudAccount) {
  state.confirm = {
    title: t('cloud.removeTitle', { name: a.name }),
    desc: t('cloud.removeDesc', { path: a.localRoot }),
    danger: true,
    ok: t('cloud.removeOk'),
    run: async () => {
      try {
        for (const tb of state.tabs) for (const p of tb.panes) if (parseCloud(p.path)?.id === a.id) go(p, state.home)
        await invoke('cloud_remove', { id: a.id })
        await refreshCloud()
      } catch (e) {
        fail(e)
      }
    },
  }
}
