import { invoke } from '@tauri-apps/api/core'
import { save } from '../util'
import { newPane, nextId, pane, state, tab } from './state'
import { load_ } from './navigation'

export function syncWatch() {
  const paths = [...new Set(state.tabs.flatMap((t) => t.panes.map((p) => p.path)))]
  invoke('watch', { paths }).catch(() => {})
}

export function newTab(path = pane()?.path ?? state.home) {
  const p = newPane(path)
  state.tabs.push({ id: nextId(), panes: [p], active: 0 })
  state.activeTab = state.tabs.length - 1
  load_(p)
  syncWatch()
  persist()
}

export function closeTab(i = state.activeTab) {
  if (state.tabs.length === 1) return
  state.tabs.splice(i, 1)
  state.activeTab = Math.min(state.activeTab, state.tabs.length - 1)
  syncWatch()
  persist()
}

export function toggleSplit() {
  const t = tab()
  if (t.panes.length > 1) {
    t.panes.splice(1 - t.active, 1)
    t.active = 0
  } else {
    const p = newPane(t.panes[0].path)
    t.panes.push(p)
    t.active = 1
    load_(p)
  }
  syncWatch()
  persist()
}

export function focusPane(i: number) {
  if (!tab().panes[i] || tab().active === i) return
  tab().active = i
  persist()
}

export function persist() {
  save(
    'orbit.session',
    { tabs: state.tabs.map((t) => ({ paths: t.panes.map((p) => p.path), active: t.active })), activeTab: state.activeTab },
  )
}
