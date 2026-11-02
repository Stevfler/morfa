import { nextTick } from 'vue'
import { commands, matches } from './commands'
import * as S from './store'

const ROW_PAGE = 15

/** List navigation shared by the file list and the filter/search inputs. Returns true if handled. */
export function navKey(p: S.Pane, e: KeyboardEvent): boolean {
  const mode = e.shiftKey ? 'extend' : 'move'
  const n = S.visible(p).length
  switch (e.key) {
    case 'ArrowDown':
      S.setCursor(p, p.cursor + 1, mode)
      break
    case 'ArrowUp':
      S.setCursor(p, p.cursor - 1, mode)
      break
    case 'PageDown':
      S.setCursor(p, p.cursor + ROW_PAGE, mode)
      break
    case 'PageUp':
      S.setCursor(p, p.cursor - ROW_PAGE, mode)
      break
    case 'Enter':
      S.open(p, S.visible(p)[p.cursor], { reveal: e.ctrlKey || e.metaKey })
      break
    default:
      return false
  }
  e.preventDefault()
  if (n) scrollCursorIntoView(p)
  return true
}

export function scrollCursorIntoView(p: S.Pane) {
  nextTick(() => document.querySelector(`[data-pane="${p.id}"]`)?.dispatchEvent(new CustomEvent('reveal-cursor')))
}

export function focusFilter(p: S.Pane) {
  nextTick(() => (document.querySelector(`[data-filter="${p.id}"]`) as HTMLInputElement | null)?.focus())
}

const inField = (e: KeyboardEvent) => {
  const t = e.target as HTMLElement
  return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable
}

/** Webview reload / print shortcuts: never let them reach the webview (would lose all state). */
export const isReloadOrPrint = (e: KeyboardEvent) => {
  if (e.key === 'F5') return true
  // AltGr reports Ctrl+Alt on Windows and types characters (¶, ®, …); leave it alone.
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.getModifierState('AltGraph')) return false
  const k = e.key.toLowerCase()
  return e.code === 'KeyR' || e.code === 'KeyP' || k === 'r' || k === 'p'
}

export function onGlobalKey(e: KeyboardEvent) {
  // Always block before any early return; the app's own F5 "Refresh" still runs below when allowed.
  if (isReloadOrPrint(e)) e.preventDefault()
  if (S.state.palette || S.state.batchRename || S.state.confirm || S.state.quickLook) return
  const p = S.pane()
  const field = inField(e)

  // These work everywhere, even while typing in a field.
  for (const id of ['palette', 'jump', 'search', 'path', 'newtab', 'closetab', 'split']) {
    const c = commands.find((c) => c.id === id)!
    if (matches(e, c.keys!)) {
      e.preventDefault()
      c.run()
      if (id === 'search') focusFilter(S.pane())
      return
    }
  }
  if (e.ctrlKey && /^Digit[1-9]$/.test(e.code)) {
    e.preventDefault()
    S.state.activeTab = Math.min(Number(e.code.slice(5)) - 1, S.state.tabs.length - 1)
    return
  }
  if (field) return

  if (navKey(p, e)) return
  if (e.key === 'Tab' && S.tab().panes.length > 1) {
    e.preventDefault()
    S.focusPane(1 - S.tab().active)
    return
  }
  if (e.key === 'Backspace' || (e.altKey && e.key === 'ArrowUp')) {
    e.preventDefault()
    return S.up(p)
  }
  if (e.key === 'Home' && !e.altKey) return S.setCursor(p, 0, e.shiftKey ? 'extend' : 'move'), scrollCursorIntoView(p)
  if (e.key === 'End') return S.setCursor(p, S.visible(p).length - 1, e.shiftKey ? 'extend' : 'move'), scrollCursorIntoView(p)
  if (e.key === 'Escape') {
    if (p.search) return S.stopSearch(p)
    if (p.filter) return (p.filter = '')
    if (S.state.clipboard) return (S.state.clipboard = null)
    return p.selected.clear()
  }
  if (e.key === 'Insert') {
    // Commander-style marking: toggle and step down, keeping other marks.
    S.setCursor(p, p.cursor, 'toggle')
    p.cursor = Math.min(p.cursor + 1, S.visible(p).length - 1)
    return scrollCursorIntoView(p)
  }

  for (const c of commands) {
    if (c.keys && matches(e, c.keys)) {
      e.preventDefault()
      c.run()
      return
    }
  }
  if (e.altKey && e.key === 'ArrowLeft') return S.back(p)
  if (e.altKey && e.key === 'ArrowRight') return S.forward(p)

  // Type-to-filter: just start typing a name.
  if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== ' ') {
    e.preventDefault()
    if (p.search) p.search.query += e.key
    else p.filter += e.key
    p.cursor = 0
    focusFilter(p)
  }
}
