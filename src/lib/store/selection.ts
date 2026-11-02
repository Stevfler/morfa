import { pane } from './state'
import { visible } from './view'
import { toast } from './toast'
import type { Pane } from './types'
import { t } from '../i18n'

export function setCursor(p: Pane, i: number, mode: 'move' | 'extend' | 'toggle' = 'move') {
  const n = visible(p).length
  if (!n) return
  i = Math.max(0, Math.min(n - 1, i))
  const list = visible(p)
  if (mode === 'move') {
    p.selected.clear()
    p.anchor = i
  } else if (mode === 'extend') {
    p.selected.clear()
    const [a, b] = [Math.min(p.anchor, i), Math.max(p.anchor, i)]
    for (let k = a; k <= b; k++) p.selected.add(list[k].path)
  } else {
    const path = list[i].path
    p.selected.has(path) ? p.selected.delete(path) : p.selected.add(path)
    p.anchor = i
  }
  p.cursor = i
}

export function selectAll(p: Pane = pane()) {
  visible(p).forEach((e) => p.selected.add(e.path))
}

/** Select by glob-ish pattern, e.g. "*.jpg" or "draft". */
export function selectPattern(p: Pane, pattern: string) {
  const re = new RegExp('^' + pattern.split('*').map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$', 'i')
  const plain = !pattern.includes('*')
  let n = 0
  p.selected.clear()
  for (const e of visible(p)) {
    if (plain ? e.name.toLowerCase().includes(pattern.toLowerCase()) : re.test(e.name)) {
      p.selected.add(e.path)
      n++
    }
  }
  toast({ title: t('selection.selected', { count: n }), desc: t('selection.matching', { pattern }), tone: 'info' }, 2500)
}
