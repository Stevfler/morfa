import type { IconName } from './icons'
import { openSettings } from './settings'
import * as S from './store'
import { canQuickLook, isMac, otherMod, primaryMod, prettyKeys } from './util'

export type Cmd = {
  id: string
  title: string
  group: string
  icon: IconName
  /** Shown to the user, e.g. "Ctrl+Shift+N" */
  keys?: string
  /** Extra binding that also triggers the command (not displayed). */
  alt?: string
  run: () => unknown
}

export { prettyKeys }

export const commands: Cmd[] = [
  { id: 'palette', title: 'Command palette', group: 'General', icon: 'command', keys: 'Ctrl+K', run: () => (S.state.palette = 'all') },
  { id: 'jump', title: 'Jump to folder…', group: 'Navigate', icon: 'zap', keys: 'Ctrl+P', run: () => (S.state.palette = 'folders') },
  { id: 'search', title: 'Search in this folder and subfolders', group: 'Navigate', icon: 'search', keys: 'Ctrl+F', run: () => S.openSearch() },
  { id: 'path', title: 'Type a path', group: 'Navigate', icon: 'link', keys: 'Ctrl+L', run: () => (S.state.editPath = true) },
  { id: 'up', title: 'Go to parent folder', group: 'Navigate', icon: 'arrow-up', keys: 'Alt+↑', run: () => S.up() },
  { id: 'back', title: 'Go back', group: 'Navigate', icon: 'chevron-left', keys: 'Alt+←', run: () => S.back() },
  { id: 'forward', title: 'Go forward', group: 'Navigate', icon: 'chevron-right', keys: 'Alt+→', run: () => S.forward() },
  { id: 'home', title: 'Go home', group: 'Navigate', icon: 'folder-open', keys: 'Alt+Home', run: () => S.go(S.pane(), S.state.home) },
  { id: 'refresh', title: 'Refresh', group: 'Navigate', icon: 'refresh', keys: 'F5', run: () => S.refreshAll() },

  { id: 'newdir', title: 'New folder', group: 'Create', icon: 'folder-plus', keys: 'Ctrl+Shift+N', run: () => S.create('dir') },
  { id: 'newfile', title: 'New empty file', group: 'Create', icon: 'file-plus', keys: 'Ctrl+Alt+N', run: () => S.create('file') },

  { id: 'copy', title: 'Copy', group: 'Edit', icon: 'copy', keys: 'Ctrl+C', run: () => S.copyToClipboard('copy') },
  { id: 'cut', title: 'Cut', group: 'Edit', icon: 'scissors', keys: 'Ctrl+X', run: () => S.copyToClipboard('move') },
  { id: 'paste', title: 'Paste here', group: 'Edit', icon: 'paste', keys: 'Ctrl+V', run: () => S.paste() },
  { id: 'rename', title: 'Rename (several selected → batch rename)', group: 'Edit', icon: 'pencil', keys: 'F2', run: () => S.startRename() },
  { id: 'trash', title: 'Move to Trash', group: 'Edit', icon: 'trash', keys: isMac ? 'Ctrl+Backspace' : 'Del', alt: 'Del', run: () => S.trash() },
  { id: 'delete', title: 'Delete forever', group: 'Edit', icon: 'x', keys: isMac ? 'Ctrl+Shift+Backspace' : 'Shift+Del', alt: 'Shift+Del', run: () => S.deleteForever() },
  { id: 'undo', title: 'Undo', group: 'Edit', icon: 'undo', keys: 'Ctrl+Z', run: () => S.undo() },
  { id: 'redo', title: 'Redo', group: 'Edit', icon: 'redo', keys: 'Ctrl+Shift+Z', run: () => S.redo() },
  { id: 'selectall', title: 'Select all', group: 'Edit', icon: 'check-all', keys: 'Ctrl+A', run: () => S.selectAll() },
  { id: 'copypath', title: 'Copy path', group: 'Edit', icon: 'link', keys: 'Ctrl+Shift+C', run: () => S.copyPath() },

  {
    id: 'copyother',
    title: 'Copy to the other pane',
    group: 'Dual pane',
    icon: 'copy',
    keys: 'F7',
    run: () => {
      const o = S.otherPane()
      o ? S.transfer(S.targets().map((e) => e.path), o.path, 'copy') : S.toggleSplit()
    },
  },
  {
    id: 'moveother',
    title: 'Move to the other pane',
    group: 'Dual pane',
    icon: 'arrow-swap',
    keys: 'F8',
    run: () => {
      const o = S.otherPane()
      o ? S.transfer(S.targets().map((e) => e.path), o.path, 'move') : S.toggleSplit()
    },
  },
  { id: 'split', title: 'Toggle dual pane', group: 'View', icon: 'columns', keys: 'Ctrl+\\', run: () => S.toggleSplit() },
  // Space is handled as hold-to-preview in keys.ts; from the palette it stays open until Esc/Space.
  { id: 'quicklook', title: 'Quick Look (hold Space)', group: 'View', icon: 'quicklook', keys: 'Space', run: () => (S.state.quickLook = canQuickLook(S.visible(S.pane())[S.pane().cursor])) },
  { id: 'settings', title: 'Settings', group: 'View', icon: 'settings', keys: 'Ctrl+,', run: () => openSettings() },
  { id: 'preview', title: 'Toggle info panel', group: 'View', icon: 'panel-right', keys: 'Ctrl+I', run: () => (S.state.preview = !S.state.preview) },
  { id: 'hidden', title: 'Show / hide hidden files', group: 'View', icon: 'eye', keys: 'Ctrl+H', run: () => (S.state.showHidden = !S.state.showHidden) },
  { id: 'sizes', title: 'Calculate folder sizes (find what eats space)', group: 'View', icon: 'gauge', keys: 'Ctrl+Shift+S', run: () => S.calcFolderSizes() },
  { id: 'newtab', title: 'New tab', group: 'Tabs', icon: 'plus', keys: 'Ctrl+T', run: () => S.newTab() },
  { id: 'closetab', title: 'Close tab', group: 'Tabs', icon: 'x', keys: 'Ctrl+W', run: () => S.closeTab() },
  { id: 'bookmark', title: 'Add / remove folder from Favorites', group: 'Folder', icon: 'star', keys: 'Ctrl+D', run: () => S.toggleBookmark() },
  { id: 'terminal', title: 'Open terminal here', group: 'Folder', icon: 'terminal', keys: 'Ctrl+`', run: () => S.openTerminal() },
]

export const cmd = (id: string) => commands.find((c) => c.id === id)!

/** "Ctrl+Shift+N" → does this KeyboardEvent match? "Ctrl" means the primary modifier (⌘ on macOS). */
export function matches(e: KeyboardEvent, keys: string) {
  if (otherMod(e)) return false
  const parts = keys.split('+')
  const key = parts.pop()!.toLowerCase()
  const want = { ctrl: parts.includes('Ctrl'), shift: parts.includes('Shift'), alt: parts.includes('Alt') }
  const ctrl = primaryMod(e)
  if (ctrl !== want.ctrl || e.altKey !== want.alt) return false
  // Shift is implied for symbols on some layouts; only enforce it when the binding names it or for letters.
  if (want.shift !== e.shiftKey && (want.shift || /^[a-z]$/.test(key) || key.length > 1)) return false
  const map: Record<string, string> = { '↑': 'arrowup', '←': 'arrowleft', '→': 'arrowright', del: 'delete', space: ' ' }
  const k = map[key] ?? key
  // Use the physical key for letters so shortcuts work on any keyboard layout (e.g. Cyrillic).
  if (/^[a-z]$/.test(k)) return e.code === `Key${k.toUpperCase()}`
  if (k === '\\') return e.code === 'Backslash'
  if (k === '`') return e.code === 'Backquote'
  return e.key.toLowerCase() === k
}

/** Matches the command's main or alternate binding. */
export const matchesCmd = (e: KeyboardEvent, c: Cmd) => (!!c.keys && matches(e, c.keys)) || (!!c.alt && matches(e, c.alt))
