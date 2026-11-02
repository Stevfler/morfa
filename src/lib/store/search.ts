import { invoke } from '@tauri-apps/api/core'
import { pane, state } from './state'
import type { Pane } from './types'

let seq = Date.now()

export async function startSearch(p: Pane, query: string) {
  if (!query.trim()) {
    if (p.search) {
      p.search.query = query
      p.search.results = []
      p.search.done = true
    }
    return invoke('cancel_search')
  }
  // Claim the id before invoking so early batches match, and keep the live query.
  const id = ++seq
  if (p.search) Object.assign(p.search, { id, results: [], done: false, scanned: 0 })
  else p.search = { query, id, results: [], done: false, scanned: 0 }
  p.cursor = 0
  p.selected.clear()
  return invoke('search', { root: p.path, query, hidden: state.showHidden, id })
}

export function openSearch(p: Pane = pane()) {
  if (!p.search) p.search = { query: '', id: -1, results: [], done: true, scanned: 0 }
}

export function stopSearch(p: Pane = pane()) {
  p.search = null
  p.cursor = 0
  p.selected.clear()
  invoke('cancel_search')
}
