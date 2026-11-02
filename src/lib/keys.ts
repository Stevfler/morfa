import { nextTick } from 'vue'
import { settingsOpen } from './settings'
import { commands, matches, matchesCmd } from './commands'
import { canQuickLook, kindOf, primaryMod } from './util'
import { imageUrl } from '../composables/useImageUrl'
import * as S from './store'
import { drives } from '../components/sidebar/drives'

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
      if (e.altKey) return false // Alt+Enter = Properties
      S.open(p, S.visible(p)[p.cursor], { reveal: primaryMod(e) })
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

/** Any dialog that owns the keyboard: global shortcuts stay off while one is open. */
export const anyModalOpen = () =>
  !!(S.state.palette || S.state.batchRename || S.state.properties || S.state.confirm || S.state.quickLook || settingsOpen.value || drives.unlock) ||
  // Any open reka dialog (covers future ones automatically).
  !!document.querySelector('[role=dialog][data-state=open],[role=alertdialog][data-state=open]')

// Space on these activates them; don't turn it into Quick Look.
const SPACE_TARGETS = 'button,[role=button],[role=switch],[role=radio],[role=tab],[role=checkbox],select,a,[contenteditable]'

function holdSpace(e: KeyboardEvent): boolean {
  if (e.key !== ' ' || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return false
  if ((e.target as HTMLElement).closest?.(SPACE_TARGETS)) return false
  e.preventDefault()
  if (e.repeat) return true
  const p = S.pane()
  const e2 = S.visible(p)[p.cursor]
  if (canQuickLook(e2)) {
    // Open once the image is decoded (or after a short cap) so it appears whole instead of popping in.
    const show = () => (S.state.quickLook = true)
    if (kindOf(e2) === 'image') Promise.race([imageUrl(e2.path), new Promise((r) => setTimeout(r, 300))]).then(show)
    else show()
  }
  return true
}

export function onGlobalKey(e: KeyboardEvent) {
  // Always block before any early return; the app's own F5 "Refresh" still runs below when allowed.
  if (isReloadOrPrint(e)) e.preventDefault()
  if (anyModalOpen()) return
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
  if (!field && primaryMod(e) && !e.altKey && /^Digit[1-9]$/.test(e.code)) {
    e.preventDefault()
    S.state.activeTab = Math.min(Number(e.code.slice(5)) - 1, S.state.tabs.length - 1)
    return
  }
  if (field) return

  if (navKey(p, e)) return
  // Only steal Tab from the file list / nothing focused, so Tab still walks sidebar/toolbar/function bar.
  const a = document.activeElement
  const inList = !a || a === document.body || !!a.closest('[data-pane]')
  if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey && inList && S.tab().panes.length > 1) {
    e.preventDefault()
    S.focusPane(1 - S.tab().active)
    return
  }
  if ((e.key === 'Backspace' && !primaryMod(e) && !e.shiftKey) || (e.altKey && e.key === 'ArrowUp')) {
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

  if (holdSpace(e)) return
  for (const c of commands) {
    if (matchesCmd(e, c)) {
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
