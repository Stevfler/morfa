import { ref } from 'vue'
import * as S from '../lib/store'
import type { Entry, Pane } from '../lib/store'

const MIME = 'application/x-orbit-paths'

export function useDragDrop(p: Pane) {
  const dropTarget = ref<string | null>(null)

  function dragStart(ev: DragEvent, e: Entry, list: Entry[]) {
    if (!p.selected.has(e.path)) S.setCursor(p, list.indexOf(e), 'move')
    const paths = S.targets(p).map((x) => x.path)
    ev.dataTransfer!.setData(MIME, JSON.stringify(paths))
    ev.dataTransfer!.setData('text/plain', paths.join('\n'))
    ev.dataTransfer!.effectAllowed = 'copyMove'
  }
  function dragOver(ev: DragEvent, dest: string) {
    if (!ev.dataTransfer?.types.includes(MIME)) return
    ev.preventDefault()
    ev.dataTransfer.dropEffect = ev.ctrlKey ? 'copy' : 'move'
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
    S.transfer(paths, dest, ev.ctrlKey ? 'copy' : 'move')
  }

  return { dropTarget, dragStart, dragOver, dragLeave, drop }
}

export type DragDrop = ReturnType<typeof useDragDrop>
