import { watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { load, save } from '../util'
import { newPane, nextId, state } from './state'
import { go, load_ } from './navigation'
import { persist, syncWatch } from './tabs'
import { startSearch } from './search'
import type { Entry, Place } from './types'

let refreshTimer: ReturnType<typeof setTimeout> | undefined
const dirty = new Set<string>()

export async function init() {
  state.home = await invoke<string>('home_dir').catch(() => '/')
  const session = load<{ tabs: { paths: string[]; active: number }[]; activeTab: number } | null>('orbit.session', null)
  if (session?.tabs.length) {
    state.tabs = session.tabs.map((t) => ({ id: nextId(), panes: t.paths.map(newPane), active: t.active }))
    state.activeTab = Math.min(session.activeTab, state.tabs.length - 1)
  } else {
    state.tabs = [{ id: nextId(), panes: [newPane(state.home)], active: 0 }]
  }
  for (const t of state.tabs) for (const p of t.panes) load_(p).then(() => {
      if (p.error) go(p, state.home, { history: false })
    })
  syncWatch()
  state.places = await invoke<Place[]>('places')

  await listen<{ id: number; entries: Entry[]; done: boolean; scanned: number }>('search', ({ payload }) => {
    for (const t of state.tabs)
      for (const p of t.panes)
        if (p.search?.id === payload.id) {
          p.search.results.push(...payload.entries)
          p.search.done = payload.done
          p.search.scanned = payload.scanned
        }
  })
  await listen<{ id: number; done: number; total: number; current: string }>('op-progress', ({ payload }) => {
    const op = state.ops.find((o) => o.id === payload.id)
    if (op) Object.assign(op, { done: payload.done, total: payload.total, current: payload.current })
  })
  await listen<string[]>('fs-changed', ({ payload }) => {
    payload.forEach((d) => dirty.add(d))
    clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      for (const t of state.tabs) for (const p of t.panes) if (!p.search && dirty.has(p.path)) load_(p)
      dirty.clear()
    }, 150)
  })

  watch(() => state.bookmarks, (v) => save('orbit.bookmarks', v), { deep: true })
  watch(() => state.showHidden, (v) => {
    save('orbit.hidden', v)
    // Re-run active searches so results honour the new hidden-files setting.
    for (const t of state.tabs) for (const p of t.panes) if (p.search?.query.trim()) startSearch(p, p.search.query)
  })
  watch(() => state.preview, (v) => save('orbit.preview', v))
  watch(() => state.activeTab, persist)
}
