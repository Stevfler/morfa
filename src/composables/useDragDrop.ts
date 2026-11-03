import { ref } from 'vue'
import * as S from '../lib/store'
import type { Entry, Pane } from '../lib/store'
import { copyMod } from '../lib/util'

const MIME = 'application/x-morfa-paths'
/** Only the dragged directories (Favorites accept folders only). */
export const DIRS_MIME = 'application/x-morfa-dirs'

/** Small label as the drag image; by default the browser snapshots the whole (pane-wide) row. */
function setGhost(ev: DragEvent, label: string) {
  const el = document.createElement('div')
  el.textContent = label
  el.style.cssText = 'position:fixed;top:-1000px;left:0;max-width:260px;padding:4px 10px;border-radius:6px;font:12px system-ui,sans-serif;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;background:var(--color-raised);color:var(--color-fg);box-shadow:0 2px 8px rgb(0 0 0/.3)'
  document.body.appendChild(el)
  ev.dataTransfer!.setDragImage(el, -12, -12)
  setTimeout(() => el.remove())
}

export function useDragDrop(p: Pane) {
  const dropTarget = ref<string | null>(null)

  function dragStart(ev: DragEvent, e: Entry, list: Entry[]) {
    if (!p.selected.has(e.path)) S.setCursor(p, list.indexOf(e), 'move')
    const t = S.targets(p)
    const paths = t.map((x) => x.path)
    ev.dataTransfer!.setData(DIRS_MIME, JSON.stringify(t.filter((x) => x.isDir).map((x) => x.path)))
    ev.dataTransfer!.setData(MIME, JSON.stringify(paths))
    ev.dataTransfer!.setData('text/plain', paths.join('\n'))
    ev.dataTransfer!.effectAllowed = 'copyMove'
    setGhost(ev, t.length > 1 ? `${t[0].name} +${t.length - 1}` : t[0]?.name ?? '')
  }
  function dragOver(ev: DragEvent, dest: string) {
    if (!ev.dataTransfer?.types.includes(MIME)) return
    ev.preventDefault()
    ev.dataTransfer.dropEffect = copyMod(ev) ? 'copy' : 'move'
    dropTarget.value = dest
  }
  function dragLeave() {
    dropTarget.value = null
  }
  function drop(ev: DragEvent, dest: string) {
    dropTarget.value = null
    const data = ev.dataTransfer?.getData(MIME)
    if (!data) return
    const paths = (JSON.parse(data) as string[]).filter((x) => x !== dest)
    S.transfer(paths, dest, copyMod(ev) ? 'copy' : 'move')
  }

  return { dropTarget, dragStart, dragOver, dragLeave, drop }
}

export type DragDrop = ReturnType<typeof useDragDrop>
