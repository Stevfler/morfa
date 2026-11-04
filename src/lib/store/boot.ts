import { watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { load, save } from '../util'
import { newPane, nextId, state } from './state'
import { go, load_ } from './navigation'
import { persist, syncWatch } from './tabs'
import { startSearch } from './search'
import { isNotResponding, type Entry, type Place } from './types'
import { settings } from '../settings'
import { initCloud } from '../cloud'

let refreshTimer: ReturnType<typeof setTimeout> | undefined
const dirty = new Set<string>()

export async function init() {
  state.home = await invoke<string>('home_dir').catch(() => '/')
  // Accounts first: mirrored folders get their sync marks on the very first load.
  await initCloud().catch(() => {})
  const session = load<{ tabs: { paths: string[]; active: number }[]; activeTab: number } | null>('morfa.session', null)
  if (session?.tabs.length) {
    state.tabs = session.tabs.map((t) => ({ id: nextId(), panes: t.paths.map(newPane), active: t.active }))
    state.activeTab = Math.min(session.activeTab, state.tabs.length - 1)
  } else {
    state.tabs = [{ id: nextId(), panes: [newPane(state.home)], active: 0 }]
  }
  for (const t of state.tabs) for (const p of t.panes) load_(p).then(() => {
      // An outage isn't a missing folder: keep the saved path, with Retry in the pane.
      if (p.error && !isNotResponding(p.error)) go(p, state.home, { history: false })
    })
  syncWatch()
  // Not awaited: a slow answer must not hold up the listeners below.
  invoke<Place[]>('places').then((pl) => (state.places = pl)).catch((e) => console.warn('places:', e))

  await listen<{ id: number; entries: Entry[]; done: boolean; scanned: number }>('search', ({ payload }) => {
    for (const t of state.tabs)
      for (const p of t.panes)
        if (p.search?.id === payload.id) {
          p.search.results.push(...payload.entries)
          p.search.done = payload.done
          p.search.scanned = payload.scanned
        }
  })
  await listen<{ id: number; done: number; total: number; current: string; part?: number | null }>('op-progress', ({ payload }) => {
    const op = state.ops.find((o) => o.id === payload.id)
    if (op) Object.assign(op, { done: payload.done, total: payload.total, current: payload.current, part: payload.part })
  })
  await listen<string[]>('fs-changed', ({ payload }) => {
    payload.forEach((d) => dirty.add(d))
    clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      for (const t of state.tabs) for (const p of t.panes) if (!p.search && dirty.has(p.path)) load_(p)
      dirty.clear()
    }, 150)
  })

  // Shares aren't watched: catch up on what changed there when the window comes back.
  let lastFocus = 0
  await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (!focused || Date.now() - lastFocus < 5000) return
    lastFocus = Date.now()
    for (const t of state.tabs) for (const p of t.panes) if (p.remote && !p.search && !p.loading && !p.error) load_(p)
  })

  watch(() => state.bookmarks, (v) => save('morfa.bookmarks', v), { deep: true })
  const rerunSearches = () => {
    for (const t of state.tabs) for (const p of t.panes) if (p.search?.query.trim()) startSearch(p, p.search.query)
  }
  watch(() => settings.respectGitignore, rerunSearches)
  watch(() => state.showHidden, (v) => {
    save('morfa.hidden', v)
    // Re-run active searches so results honour the new hidden-files setting.
    for (const t of state.tabs) for (const p of t.panes) if (p.search?.query.trim()) startSearch(p, p.search.query)
  })
  watch(() => state.preview, (v) => save('morfa.preview', v))
  watch(() => state.activeTab, persist)
}
