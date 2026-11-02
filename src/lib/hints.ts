import * as S from './store'
import type { Pane } from './store'
import { cmd } from './commands'

/** Keyboard hints for the bottom bar: [keys, text] pairs for what makes sense in `p` right now. */
export function hintsFor(p: Pane, split: boolean): [string, string][] {
  const state = S.state
  if (state.clipboard) return [['Ctrl+V', `paste ${state.clipboard.paths.length} here`], ['Esc', 'forget clipboard']]
  if (p.search) return [['Enter', 'open'], ['Ctrl+Enter', 'show in folder'], ['Esc', 'exit search']]
  if (p.selected.size > 1) return [['F2', 'batch rename'], ['Ctrl+C', 'copy'], [cmd('trash').keys!, 'trash'], ...(split ? [[cmd('copyother').keys!, 'copy → other pane'] as [string, string]] : [])]
  if (p.filter) return [['↑↓', 'pick'], ['Enter', 'open'], ['Esc', 'clear filter']]
  return [['type', 'to filter'], ['Ctrl+F', 'deep search'], ['Space', 'preview'], ['Ctrl+K', 'all actions']]
}
