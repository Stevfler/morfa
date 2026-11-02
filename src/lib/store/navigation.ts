import { invoke } from '@tauri-apps/api/core'
import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener'
import { parent, save } from '../util'
import { pane, state } from './state'
import { visible } from './view'
import { fail } from './toast'
import { stopSearch } from './search'
import { persist, syncWatch } from './tabs'
import { isSpecial, type Entry, type Pane, type Place } from './types'

// Per-pane load token: only the latest load_ for a pane may apply its result.
const loadSeq = new WeakMap<Pane, number>()

export async function load_(p: Pane) {
  const seq = (loadSeq.get(p) ?? 0) + 1
  loadSeq.set(p, seq)
  const stale = () => loadSeq.get(p) !== seq
  if (isSpecial(p.path)) {
    // Special views (Drives overview) render their own content; nothing to list.
    Object.assign(p, { entries: [], error: null, loading: false })
    return
  }
  p.loading = true
  try {
    const entries = await invoke<Entry[]>('list_dir', { path: p.path })
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

export function up(p: Pane = pane()) {
  const par = isSpecial(p.path) ? null : parent(p.path)
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
  save('orbit.frecency', state.frecency)
}
export function frecencyScore(path: string) {
  const f = state.frecency[path]
  if (!f) return 0
  const hours = (Date.now() - f.t) / 3.6e6
  const w = hours < 1 ? 4 : hours < 24 ? 2 : hours < 168 ? 1 : 0.25
  return f.n * w
}
