import { invoke } from '@tauri-apps/api/core'
import { openPath, openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { parent, save } from '../util'
import { pane, state } from './state'
import { visible } from './view'
import { fail, toast } from './toast'
import { stopSearch } from './search'
import { persist, syncWatch } from './tabs'
import { cloudPath, isSpecial, parseCloud, type Entry, type Pane, type Place } from './types'
import { t } from '../i18n'

// Per-pane load token: only the latest load_ for a pane may apply its result.
const loadSeq = new WeakMap<Pane, number>()

export async function load_(p: Pane) {
  const seq = (loadSeq.get(p) ?? 0) + 1
  loadSeq.set(p, seq)
  const stale = () => loadSeq.get(p) !== seq
  const cloud = parseCloud(p.path)
  if (isSpecial(p.path) && !cloud) {
    // Special views (Drives overview) render their own content; nothing to list.
    Object.assign(p, { entries: [], error: null, loading: false })
    return
  }
  p.loading = true
  try {
    const entries = cloud
      ? await invoke<Entry[]>('cloud_list', { id: cloud.id, rel: cloud.rel })
      : await invoke<Entry[]>('list_dir', { path: p.path })
    if (stale()) return
    if (!cloud) await markMirrored(p.path, entries)
    if (stale()) return
    p.entries = entries
    p.error = null
    // Drop selections that no longer exist.
    const alive = new Set(entries.map((e) => e.path))
    for (const s of [...p.selected]) if (!alive.has(s)) p.selected.delete(s)
    if (p.focusAfterLoad) {
      const i = visible(p).findIndex((e) => e.path === p.focusAfterLoad)
      if (i >= 0) p.cursor = i
      p.focusAfterLoad = undefined
    }
    p.cursor = Math.min(p.cursor, Math.max(0, visible(p).length - 1))
  } catch (e) {
    if (stale()) return
    p.error = String(e)
    p.entries = []
  } finally {
    if (!stale()) p.loading = false
  }
}

/** Inside a cloud account's local folder: attach sync marks to the entries. */
async function markMirrored(dir: string, entries: Entry[]) {
  if (!state.cloud.some((a) => within(dir, a.localRoot))) return
  const marks = await invoke<Record<string, string>>('cloud_marks', { dir, names: entries.map((e) => e.name) }).catch(() => ({}) as Record<string, string>)
  for (const e of entries) e.cloud = marks[e.name] ?? ''
}
export const within = (path: string, root: string) =>
  path === root || path.startsWith(root.replace(/[\\/]+$/, '') + (root.includes('\\') && !root.includes('/') ? '\\' : '/'))

export async function go(p: Pane, path: string, opts: { history?: boolean; focus?: string } = {}) {
  if (p.search) stopSearch(p)
  if (path !== p.path && opts.history !== false) {
    p.back.push(p.path)
    p.fwd = []
  }
  p.path = path
  p.filter = ''
  p.selected.clear()
  p.cursor = 0
  p.focusAfterLoad = opts.focus
  if (!isSpecial(path)) bump(path)
  await load_(p)
  syncWatch()
  persist()
}

function parentOf(path: string) {
  const c = parseCloud(path)
  if (c) return c.rel ? cloudPath(c.id, c.rel.split('/').slice(0, -1).join('/')) : null
  return isSpecial(path) ? null : parent(path)
}
export const canUp = (p: Pane) => !!parentOf(p.path)
export function up(p: Pane = pane()) {
  const par = parentOf(p.path)
  if (par) go(p, par, { focus: p.path })
}
export function back(p: Pane = pane()) {
  const prev = p.back.pop()
  if (!prev) return
  p.fwd.push(p.path)
  go(p, prev, { history: false, focus: undefined })
}
export function forward(p: Pane = pane()) {
  const next = p.fwd.pop()
  if (!next) return
  p.back.push(p.path)
  go(p, next, { history: false })
}

export async function open(p: Pane, e: Entry | undefined, opts: { reveal?: boolean } = {}) {
  if (!e) return
  if (e.isDir && !opts.reveal) return go(p, e.path)
  const cloud = parseCloud(e.path)
  if (cloud) return openCloudFile(cloud.id, cloud.rel, e)
  if (p.search || opts.reveal) {
    // From search results: jump into the containing folder with the item focused.
    const dir = parent(e.path)
    if (dir) return go(p, dir, { focus: e.path })
  }
  try {
    await openPath(e.path)
  } catch (err) {
    fail(err)
  }
}

/** Open a file from the cloud view: synced → local copy; otherwise download (and keep syncing) first. */
async function openCloudFile(id: string, rel: string, e: Entry) {
  try {
    if (e.link) return await openUrl(e.link)
    let local = e.local
    if (e.cloud !== 'synced' || !local) {
      toast({ title: t('cloud.downloading', { name: e.name }), tone: 'info' }, 2500)
      local = await invoke<string>('cloud_fetch', { id, rel })
    }
    await openPath(local)
  } catch (err) {
    fail(err)
  }
}

export const reveal = (path: string) => revealItemInDir(path).catch(fail)

export function refreshAll() {
  for (const t of state.tabs) for (const p of t.panes) if (!p.search) load_(p)
  invoke<Place[]>('places').then((pl) => (state.places = pl))
}

// Frecency: folders you visit often & recently float to the top of "Jump to".
export function bump(path: string) {
  const f = state.frecency[path] ?? { n: 0, t: 0 }
  state.frecency[path] = { n: f.n + 1, t: Date.now() }
  const keys = Object.keys(state.frecency)
  if (keys.length > 400) {
    keys.sort((a, b) => frecencyScore(a) - frecencyScore(b)).slice(0, 100).forEach((k) => delete state.frecency[k])
  }
  save('morfa.frecency', state.frecency)
}
export function frecencyScore(path: string) {
  const f = state.frecency[path]
  if (!f) return 0
  const hours = (Date.now() - f.t) / 3.6e6
  const w = hours < 1 ? 4 : hours < 24 ? 2 : hours < 168 ? 1 : 0.25
  return f.n * w
}
