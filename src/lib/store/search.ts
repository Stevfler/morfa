import { invoke } from '@tauri-apps/api/core'
import { pane, state } from './state'
import type { Entry, Pane } from './types'
import { settings } from '../settings'
import { fail } from './toast'

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
  // "#tag" lists every file carrying that tag instead of walking the folder.
  if (query.startsWith('#') && query.length > 1) {
    const results = await invoke<Entry[]>('tags_find', { tag: query.slice(1).trim() }).catch((e) => (fail(e), [] as Entry[]))
    if (p.search?.id === id) Object.assign(p.search, { results, done: true, scanned: results.length })
    return
  }
  return invoke('search', { root: p.path, query, hidden: state.showHidden, gitignore: settings.respectGitignore, id }).catch((e) => {
    if (p.search?.id === id) p.search.done = true
    fail(e)
  })
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
