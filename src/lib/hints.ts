import * as S from './store'
import type { Pane } from './store'
import { cmd } from './commands'
import { t } from './i18n'

/** Keyboard hints for the bottom bar: [keys, text] pairs for what makes sense in `p` right now. */
export function hintsFor(p: Pane, split: boolean): [string, string][] {
  const state = S.state
  if (state.clipboard) return [['Ctrl+V', t('hint.pasteHere', { count: state.clipboard.paths.length })], ['Esc', t('hint.forgetClipboard')]]
  if (p.search) return [['Enter', t('hint.open')], ['Ctrl+Enter', t('hint.showInFolder')], ['Esc', t('hint.exitSearch')]]
  if (p.selected.size > 1) return [...(S.canBatchRename(p) ? [['F2', t('hint.batchRename')] as [string, string]] : []), ['Ctrl+C', t('hint.copy')], [cmd('trash').keys!, t('hint.trash')], ...(split ? [[cmd('copyother').keys!, t('hint.copyOther')] as [string, string]] : [])]
  if (p.filter) return [['↑↓', t('hint.pick')], ['Enter', t('hint.open')], ['Esc', t('hint.clearFilter')]]
  return [[t('hint.type'), t('hint.toFilter')], ['Ctrl+F', t('hint.deepSearch')], ['Ctrl+Shift+U', t('hint.duplicates')], ['Space', t('hint.preview')], ['Ctrl+K', t('hint.allActions')]]
}
