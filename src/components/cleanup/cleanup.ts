import { reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { settings } from '../../lib/settings'
import { toast, fail } from '../../lib/store'
import { basename, fmtSize } from '../../lib/util'

/** Mirrors `cleanup::index::Node` / `categories::{Item, Category}` in Rust. */
export type SpaceNode = { path: string; name: string; size: number; files: number; kind: 'dir' | 'file' | 'rest' }
export type Risk = 'safe' | 'review'
export type CleanItem = { path: string; size: number; category: string; risk: Risk; protected: boolean }
export type Category = { id: string; label: string; desc: string; total: number; items: CleanItem[] }
type Report = { root: SpaceNode; children: SpaceNode[]; categories: Category[]; elapsed_ms: number }
type DeleteReport = { removed: string[]; rejected: [string, string][]; freed: number }

export const cleanup = reactive({
  warn: false,
  open: false,
  root: '',
  name: '',
  scanning: false,
  started: 0,
  id: 0,
  files: 0,
  bytes: 0,
  error: '',
  report: null as Report | null,
  /** Space-map drill path: crumbs[0] is the root node. */
  crumbs: [] as SpaceNode[],
  children: [] as SpaceNode[],
  /** path → size of every checked item (deduped across categories). */
  selected: new Map<string, number>(),
  busy: false,
})

let unlisten: UnlistenFn | null = null

/** Open the clean-up tool for a folder or mount root (shows the expert warning first unless dismissed). */
export function openCleanup(path: string) {
  cleanup.root = path
  cleanup.name = basename(path) || path
  if (settings.cleanupWarned) start()
  else cleanup.warn = true
}

/** Called by the warning's Continue. */
export function start() {
  cleanup.warn = false
  cleanup.open = true
  void scan()
}

export async function scan() {
  const id = Date.now()
  Object.assign(cleanup, { id, scanning: true, started: performance.now(), files: 0, bytes: 0, error: '', report: null, crumbs: [], children: [] })
  cleanup.selected.clear()
  unlisten ??= await listen<{ id: number; files: number; bytes: number }>('cleanup-progress', ({ payload: p }) => {
    if (p.id === cleanup.id) Object.assign(cleanup, { files: p.files, bytes: p.bytes })
  })
  try {
    const r = await invoke<Report>('cleanup_scan', { root: cleanup.root, id })
    if (cleanup.id !== id) return
    Object.assign(cleanup, { report: r, crumbs: [r.root], children: r.children, bytes: r.root.size, files: r.root.files })
  } catch (e) {
    if (cleanup.id === id) cleanup.error = String(e) === 'cancelled' ? 'Scan cancelled.' : String(e)
  } finally {
    if (cleanup.id === id) cleanup.scanning = false
  }
}

export function cancelScan() {
  if (cleanup.scanning) void invoke('cleanup_cancel')
}

export function close() {
  cancelScan()
  cleanup.open = false
  unlisten?.()
  unlisten = null
}

export async function drill(n: SpaceNode, crumb = -1) {
  if (n.kind !== 'dir') return
  try {
    const kids = await invoke<SpaceNode[]>('cleanup_children', { path: n.path })
    cleanup.crumbs = crumb >= 0 ? cleanup.crumbs.slice(0, crumb + 1) : [...cleanup.crumbs, n]
    cleanup.children = kids
  } catch (e) {
    fail(e)
  }
}

export function toggle(i: CleanItem, on: boolean) {
  if (i.protected) return
  if (on) cleanup.selected.set(i.path, i.size)
  else cleanup.selected.delete(i.path)
}

export const selectedBytes = () => [...cleanup.selected.values()].reduce((a, b) => a + b, 0)

/** Drop removed paths (and anything inside them) from every category and recompute totals. */
function prune(removed: string[]) {
  const gone = (p: string) => removed.some((r) => p === r || p.startsWith(r + '/') || p.startsWith(r + '\\'))
  for (const c of cleanup.report?.categories ?? []) {
    c.items = c.items.filter((i) => !gone(i.path))
    c.total = c.items.reduce((a, i) => a + i.size, 0)
  }
  for (const p of removed) cleanup.selected.delete(p)
}

export async function removeSelected(permanent: boolean) {
  const paths = [...cleanup.selected.keys()]
  if (!paths.length) return
  cleanup.busy = true
  try {
    const r = await invoke<DeleteReport>('cleanup_delete', { paths, permanent })
    prune(r.removed)
    if (r.removed.length) toast({ title: `Freed ${fmtSize(r.freed)}`, desc: `${r.removed.length} item(s) ${permanent ? 'deleted' : 'moved to Trash'}`, tone: 'ok' })
    if (r.rejected.length) toast({ title: `${r.rejected.length} item(s) not removed`, desc: r.rejected.slice(0, 4).map(([p, why]) => `${basename(p)}: ${why}`).join('\n'), tone: 'error' }, 9000)
  } catch (e) {
    fail(e)
  } finally {
    cleanup.busy = false
  }
}

export async function emptyTrash() {
  cleanup.busy = true
  try {
    await invoke('cleanup_empty_trash')
    const t = cleanup.report?.categories.find((c) => c.id === 'trash')
    toast({ title: `Trash emptied`, desc: t ? `Freed ${fmtSize(t.total)}` : undefined, tone: 'ok' })
    if (t) prune(t.items.map((i) => i.path))
  } catch (e) {
    if (String(e).startsWith('UNSUPPORTED_EMPTY_TRASH:')) {
      toast({ title: "Can't empty Trash here", desc: 'Empty it from Finder instead.', tone: 'info', action: { label: 'Show Trash', run: () => void invoke('reveal_trash').catch(fail) } })
    } else fail(e)
  } finally {
    cleanup.busy = false
  }
}
