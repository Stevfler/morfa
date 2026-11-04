import { ref } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import * as S from '../lib/store'
import type { Entry, Pane } from '../lib/store'
import { copyMod, isWindows, parent } from '../lib/util'
import { webviewZoom } from '../lib/settings'
import { anyModalOpen } from '../lib/keys'
import { t } from '../lib/i18n'

// Why internal drags don't use HTML5 dnd: OS file drops need Tauri's native handler (only it gets
// real paths), and that handler swallows the webview's own dnd. Windows replaces WebView2's drop
// target outright; on macOS Tauri's handler always claims the event, so WebKit never sees it.
//
// Drop targets are marked in the templates, and both kinds of drag hit-test them:
//   data-drop="<path>"  a folder: row, crumb, sidebar item, or a whole pane
//   data-drop-pane="<id>"  on the whole-pane targets; a release there means "this pane's folder"
//   data-drop-fav       the Favorites section (internal drags of folders only)

/** Path of the folder currently highlighted as a drop target. */
export const dropTarget = ref<string | null>(null)

const THRESHOLD = 5
const EDGE = 28
const SCROLL_STEP = 8

function hit(x: number, y: number) {
  return document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop],[data-drop-fav]') ?? null
}
/** The folder to light up: pseudo-locations refuse the drop, so they never look like they accept it. */
function highlight(el: HTMLElement | null) {
  const dest = el?.dataset.drop ?? null
  dropTarget.value = dest && !S.isSpecial(dest) ? dest : null
}
/**
 * Where an internal drag would land. Null when it's a no-op: back onto the pressed row, or onto the
 * source pane's own folder, which in a search or tag pane isn't where the results live, so a sloppy
 * click must not move anything. A file row in another pane still means that pane's folder.
 */
function internalDest(el: HTMLElement | null, p: Pane, pressed: string) {
  const dest = el?.dataset.drop
  if (dest === undefined || dest === pressed) return null
  const own = String(p.id)
  if (el!.dataset.dropPane === own) return null
  const root = el!.closest<HTMLElement>('section[data-drop-pane]')?.dataset.dropPane
  if (dest === p.path && (root === undefined || root === own)) return null
  return dest
}
const isAbsolute = (path: string) => /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(path)

/**
 * Swallow the click that ends a gesture, which would otherwise change the selection. With `held`,
 * the button is still down, so that click only comes after its release.
 */
function swallowClick(held: boolean) {
  const swallow = (c: MouseEvent) => c.stopPropagation()
  const stop = () => {
    window.removeEventListener('click', swallow, true)
    window.removeEventListener('mouseup', later, true)
    window.removeEventListener('mousedown', stop, true)
  }
  const later = () => setTimeout(stop)
  window.addEventListener('click', swallow, { capture: true, once: true })
  if (!held) return later()
  window.addEventListener('mouseup', later, true)
  window.addEventListener('mousedown', stop, true) // released outside the window: a new gesture began
}
function refuse() {
  S.toast({ title: t('drop.refused'), desc: t('drop.refusedHint'), tone: 'info' }, 3000)
}

function makeGhost(label: string) {
  const el = document.createElement('div')
  el.textContent = label
  el.style.cssText = 'position:fixed;top:0;left:0;z-index:100;pointer-events:none;max-width:260px;padding:4px 10px;border-radius:6px;font:12px system-ui,sans-serif;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;background:var(--color-raised);color:var(--color-fg);box-shadow:0 2px 8px rgb(0 0 0/.3)'
  document.body.appendChild(el)
  return el
}

/**
 * Call on mousedown over a row: arms a drag of the row's selection that starts once the mouse has
 * moved a few pixels. `onStart` lets the caller drop its pending click behaviour.
 */
export function pressToDrag(ev: MouseEvent, p: Pane, e: Entry, onStart: () => void) {
  if ((ev.target as HTMLElement).closest('input')) return // renaming: the mouse selects text
  const { clientX: startX, clientY: startY } = ev
  let dragging: { paths: string[]; dirs: string[]; label: string; ghost: HTMLElement } | null = null
  let x = startX
  let y = startY
  let raf = 0

  function begin() {
    // The list may have refreshed since the press; the entry may be elsewhere in it, or gone.
    const i = S.visible(p).findIndex((x) => x.path === e.path)
    if (i < 0) return end()
    onStart()
    if (!p.selected.has(e.path)) S.setCursor(p, i, 'move')
    const sel = S.targets(p)
    const label = sel.length > 1 ? `${sel[0].name} +${sel.length - 1}` : sel[0]?.name ?? ''
    dragging = { paths: sel.map((s) => s.path), dirs: sel.filter((s) => s.isDir).map((s) => s.path), label, ghost: makeGhost(label) }
    document.documentElement.classList.add('morfa-dragging')
    raf = requestAnimationFrame(autoScroll)
  }
  /** Keep scrolling while the cursor rests near a list's top or bottom edge, as native dnd did. */
  function autoScroll() {
    const list = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-pane]')
    if (list) {
      const r = list.getBoundingClientRect()
      const dy = y < r.top + EDGE ? -SCROLL_STEP : y > r.bottom - EDGE ? SCROLL_STEP : 0
      if (dy) {
        list.scrollTop += dy
        show()
      }
    }
    raf = requestAnimationFrame(autoScroll)
  }
  function end() {
    window.removeEventListener('mousemove', onMove, true)
    window.removeEventListener('mouseup', onUp, true)
    window.removeEventListener('keydown', onKey, true)
    window.removeEventListener('blur', cancel)
    window.removeEventListener('contextmenu', cancel, true)
    if (!dragging) return
    cancelAnimationFrame(raf)
    dragging.ghost.remove()
    dragging = null
    dropTarget.value = null
    document.documentElement.classList.remove('morfa-dragging')
  }
  function onMove(m: MouseEvent) {
    if (!m.buttons) return end() // released somewhere we didn't hear about
    x = m.clientX
    y = m.clientY
    if (!dragging && Math.hypot(x - startX, y - startY) < THRESHOLD) return
    if (!dragging) begin()
    const d = dragging
    if (!d) return
    d.ghost.textContent = copyMod(m) ? `+ ${d.label}` : d.label
    d.ghost.style.transform = `translate(${x + 12}px, ${y + 12}px)`
    show()
  }
  function show() {
    const el = hit(x, y)
    highlight(el?.dataset.drop !== undefined && internalDest(el, p, e.path) === null ? null : el)
  }
  function cancel() {
    const was = !!dragging
    end()
    if (was) swallowClick(true)
  }
  function onUp(m: MouseEvent) {
    if (m.button !== 0) return cancel() // only the left button drops
    const d = dragging
    const el = d && hit(m.clientX, m.clientY)
    end()
    if (!d) return
    swallowClick(false)
    if (!el) return
    if (el.dataset.drop === undefined) {
      for (const dir of d.dirs) if (!S.state.bookmarks.includes(dir)) S.state.bookmarks.push(dir)
      return
    }
    const dest = internalDest(el, p, e.path)
    if (dest === null) return
    if (S.isSpecial(dest)) return refuse()
    S.transfer(d.paths.filter((x) => x !== dest), dest, copyMod(m) ? 'copy' : 'move')
  }
  /** While dragging the keyboard does nothing but cancel, as during a native drag. */
  function onKey(k: KeyboardEvent) {
    if (!dragging) return
    k.preventDefault()
    k.stopImmediatePropagation()
    if (k.key === 'Escape') cancel()
  }
  window.addEventListener('mousemove', onMove, true)
  window.addEventListener('mouseup', onUp, true)
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', cancel)
  window.addEventListener('contextmenu', cancel, true)
}

/** Copy files dropped from the OS into `dest`; a file already in that folder stays where it is. */
export function dropFromOs(paths: string[], dest: string) {
  if (S.isSpecial(dest)) return refuse()
  // Non-local URIs (Linux hands them over as if they were paths) can't be copied.
  const local = paths.filter(isAbsolute)
  if (local.length < paths.length) S.toast({ title: t('drop.skipped', { count: paths.length - local.length }), tone: 'info' }, 4000)
  if (!local.length) return
  // By string: a symlinked or differently-cased spelling of this folder still copies, numbered.
  const sources = local.filter((x) => x !== dest && parent(x) !== dest)
  if (!sources.length) return S.toast({ title: t('drop.alreadyHere'), tone: 'info' }, 2500)
  S.transfer(sources, dest, 'copy')
}

/**
 * wry reports client physical pixels on Windows, but points on macOS and GTK logical pixels on
 * Linux, all labelled "physical" by Tauri. Webview zoom scales CSS pixels on top of the latter;
 * on Windows it's already part of devicePixelRatio.
 */
function toCss(pos: { x: number; y: number }) {
  const scale = isWindows ? window.devicePixelRatio : webviewZoom()
  return { x: pos.x / scale, y: pos.y / scale }
}

/** Listen for files dragged in from the OS for the lifetime of the window. */
export async function listenOsDrops(): Promise<() => void> {
  if (!('__TAURI_INTERNALS__' in window)) return () => {} // plain vite: no webview
  let carrying = false
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'leave') {
      carrying = false
      dropTarget.value = null
      return
    }
    // A drag of something that isn't files (text, a page image) comes with no paths.
    if (payload.type === 'enter') carrying = payload.paths.length > 0
    if (!carrying || anyModalOpen()) {
      dropTarget.value = null
      return
    }
    const { x, y } = toCss(payload.position)
    const el = hit(x, y)
    if (payload.type !== 'drop') return highlight(el)
    carrying = false
    dropTarget.value = null
    if (el?.dataset.drop !== undefined) dropFromOs(payload.paths, el.dataset.drop)
  })
}
