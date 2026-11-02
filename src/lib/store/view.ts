import { collator, fuzzy } from '../util'
import { pane, state } from './state'
import type { Entry, Pane } from './types'

export function visible(p: Pane): Entry[] {
  if (p.search) return p.search.results
  let list = state.showHidden ? p.entries : p.entries.filter((e) => !e.hidden)
  if (p.filter) {
    // Type-to-filter keeps best matches on top.
    const scored = list.map((e) => [fuzzy(p.filter, e.name), e] as const).filter(([s]) => s >= 0)
    scored.sort((a, b) => b[0] - a[0])
    return scored.map(([, e]) => e)
  }
  const { key, asc } = p.sort
  const dir = asc ? 1 : -1
  const sizeOf = (e: Entry) => (e.isDir ? (typeof state.sizes[e.path] === 'number' ? (state.sizes[e.path] as number) : -1) : e.size)
  return [...list].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
    let c = 0
    if (key === 'size') c = sizeOf(a) - sizeOf(b)
    else if (key === 'modified') c = a.modified - b.modified
    else if (key === 'ext') c = collator.compare(a.ext, b.ext)
    return (c || collator.compare(a.name, b.name)) * dir
  })
}

export function targets(p: Pane = pane()): Entry[] {
  const list = visible(p)
  if (p.selected.size) return list.filter((e) => p.selected.has(e.path))
  return list[p.cursor] ? [list[p.cursor]] : []
}
