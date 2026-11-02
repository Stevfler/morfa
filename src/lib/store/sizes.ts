import { invoke } from '@tauri-apps/api/core'
import { watch } from 'vue'
import { basename, fmtSize } from '../util'
import { pane, state } from './state'
import { targets, visible } from './view'
import { fail, toast } from './toast'
import type { Pane } from './types'

// Size walks are tagged with a generation; cancelSizes() stops every walk started before it.
let gen = Date.now() // monotonic across webview reloads (backend keeps the last cancelled gen)
const inflight = new Map<string, number>() // path -> generation measuring it

export function cancelSizes() {
  if (!inflight.size) return
  invoke('cancel_sizes', { gen }).catch(() => {})
  gen++
}

// Leaving a folder (in any pane/tab) abandons its pending size walks.
watch(
  () => state.tabs.flatMap((t) => t.panes.map((p) => p.path)).join('\0'),
  cancelSizes,
)

/** Resolves true if all walks finished, false if they were cancelled (navigation) meanwhile. */
export async function computeSizes(paths: string[]): Promise<boolean> {
  const g = gen
  await Promise.all(
    paths.map(async (path) => {
      if (inflight.get(path) === g) return // already being measured
      inflight.set(path, g)
      state.sizes[path] = 'pending'
      try {
        const [bytes] = await invoke<[number, number]>('dir_size', { path, gen: g })
        state.sizes[path] = bytes
      } catch {
        if (inflight.get(path) === g) delete state.sizes[path] // unless a newer walk took over
      } finally {
        if (inflight.get(path) === g) inflight.delete(path)
      }
    }),
  )
  return gen === g
}

export function calcFolderSizes(p: Pane = pane()) {
  const dirs = visible(p).filter((e) => e.isDir).map((e) => e.path)
  if (!dirs.length) return
  computeSizes(dirs).then((done) => {
    if (!done) return
    p.sort = { key: 'size', asc: false }
    const total = dirs.reduce((s, d) => s + ((state.sizes[d] as number) || 0), 0)
    toast({ title: 'Folder sizes ready', desc: `Sorted biggest first · ${fmtSize(total)} in folders`, tone: 'ok' })
  })
}

export function toggleBookmark(path = pane().path) {
  const i = state.bookmarks.indexOf(path)
  if (i >= 0) {
    state.bookmarks.splice(i, 1)
    toast({ title: `Removed “${basename(path)}” from Favorites`, tone: 'info' }, 2000)
  } else {
    state.bookmarks.push(path)
    toast({ title: `Added “${basename(path)}” to Favorites`, tone: 'ok' }, 2000)
  }
}

export const openTerminal = (path = pane().path) => invoke('open_terminal', { path }).catch(fail)

export async function copyPath(p: Pane = pane()) {
  const t = targets(p)
  const text = (t.length ? t.map((e) => e.path) : [p.path]).join('\n')
  await navigator.clipboard.writeText(text)
  toast({ title: 'Path copied', desc: text.length > 80 ? text.slice(0, 80) + '…' : text, tone: 'info' }, 2000)
}
