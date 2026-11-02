import type { IconName } from './icons'
import { openCleanup } from '../components/cleanup/cleanup'
import { openSettings } from './settings'
import * as S from './store'
import { isImage, openConvert } from './convert'
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
  /** Commander-style function key. Active in `fkeyMode` ('always' by default) and shown instead of `keys` while split. */
  fkey?: string
  fkeyMode?: 'always' | 'split' | 'single'
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
  { id: 'refresh', title: 'Refresh', group: 'Navigate', icon: 'refresh', keys: 'Ctrl+R', fkey: 'F5', fkeyMode: 'single', run: () => S.refreshAll() },

  { id: 'newdir', title: 'New folder', group: 'Create', icon: 'folder-plus', keys: 'Ctrl+Shift+N', fkey: 'F7', run: () => S.create('dir') },
  { id: 'newfile', title: 'New empty file', group: 'Create', icon: 'file-plus', keys: 'Ctrl+Alt+N', run: () => S.create('file') },

  { id: 'copy', title: 'Copy', group: 'Edit', icon: 'copy', keys: 'Ctrl+C', run: () => S.copyToClipboard('copy') },
  { id: 'cut', title: 'Cut', group: 'Edit', icon: 'scissors', keys: 'Ctrl+X', run: () => S.copyToClipboard('move') },
  { id: 'paste', title: 'Paste here', group: 'Edit', icon: 'paste', keys: 'Ctrl+V', run: () => S.paste() },
  { id: 'rename', title: 'Rename (several selected → batch rename)', group: 'Edit', icon: 'pencil', keys: 'F2', run: () => S.startRename() },
  { id: 'trash', title: 'Move to Trash', group: 'Edit', icon: 'trash', keys: isMac ? 'Ctrl+Backspace' : 'Del', alt: 'Del', fkey: 'F8', run: () => S.trash() },
  { id: 'delete', title: 'Delete forever', group: 'Edit', icon: 'x', keys: isMac ? 'Ctrl+Shift+Backspace' : 'Shift+Del', alt: 'Shift+Del', fkey: 'Shift+F8', run: () => S.deleteForever() },
  { id: 'undo', title: 'Undo', group: 'Edit', icon: 'undo', keys: 'Ctrl+Z', run: () => S.undo() },
  { id: 'redo', title: 'Redo', group: 'Edit', icon: 'redo', keys: 'Ctrl+Shift+Z', run: () => S.redo() },
  { id: 'selectall', title: 'Select all', group: 'Edit', icon: 'check-all', keys: 'Ctrl+A', run: () => S.selectAll() },
  { id: 'convert', title: 'Convert images…', group: 'Edit', icon: 'image', run: () => convertTargets() },
  { id: 'copypath', title: 'Copy path', group: 'Edit', icon: 'link', keys: 'Ctrl+Shift+C', run: () => S.copyPath() },

  { id: 'copyother', title: 'Copy to the other pane', group: 'Dual pane', icon: 'copy', fkey: 'F5', fkeyMode: 'split', run: () => S.toOther('copy') },
  { id: 'moveother', title: 'Move to the other pane', group: 'Dual pane', icon: 'arrow-swap', fkey: 'F6', fkeyMode: 'split', run: () => S.toOther('move') },
  { id: 'openapp', title: 'Open with default app', group: 'Dual pane', icon: 'external-link', fkey: 'F4', fkeyMode: 'split', run: () => S.openDefault() },
  { id: 'otherpane', title: 'Switch to the other pane (Tab)', group: 'Dual pane', icon: 'columns', run: () => S.focusPane(1 - S.tab().active) },
  { id: 'split', title: 'Toggle dual pane', group: 'View', icon: 'columns', keys: 'Ctrl+\\', run: () => S.toggleSplit() },
  // Space is handled as hold-to-preview in keys.ts; from the palette it stays open until Esc/Space.
  { id: 'quicklook', title: 'Quick Look (hold Space)', group: 'View', icon: 'quicklook', keys: 'Space', fkey: 'F3', fkeyMode: 'split', run: () => (S.state.quickLook = canQuickLook(S.visible(S.pane())[S.pane().cursor])) },
  { id: 'cleanup', title: 'Clean up this drive…', group: 'View', icon: 'broom', run: () => openCleanup(mountRoot(S.pane().path)) },
  { id: 'settings', title: 'Settings', group: 'View', icon: 'settings', keys: 'Ctrl+,', run: () => openSettings() },
  { id: 'preview', title: 'Toggle info panel', group: 'View', icon: 'panel-right', keys: 'Ctrl+I', run: () => (S.state.preview = !S.state.preview) },
  { id: 'hidden', title: 'Show / hide hidden files', group: 'View', icon: 'eye', keys: 'Ctrl+H', run: () => (S.state.showHidden = !S.state.showHidden) },
  { id: 'sizes', title: 'Calculate folder sizes (find what eats space)', group: 'View', icon: 'gauge', keys: 'Ctrl+Shift+S', run: () => S.calcFolderSizes() },
  { id: 'newtab', title: 'New tab', group: 'Tabs', icon: 'plus', keys: 'Ctrl+T', run: () => S.newTab() },
  { id: 'closetab', title: 'Close tab', group: 'Tabs', icon: 'x', keys: 'Ctrl+W', run: () => S.closeTab() },
  { id: 'bookmark', title: 'Add / remove folder from Favorites', group: 'Folder', icon: 'star', keys: 'Ctrl+D', run: () => S.toggleBookmark() },
  { id: 'terminal', title: 'Open terminal here', group: 'Folder', icon: 'terminal', keys: 'Ctrl+`', run: () => S.openTerminal() },
]

// These act on the folder shown in the pane, so they do nothing on pseudo-locations like the Drives view.
const NEEDS_FOLDER = new Set(['cleanup', 'search', 'refresh', 'newdir', 'newfile', 'paste', 'selectall', 'sizes', 'bookmark', 'terminal'])
for (const c of commands) {
  if (!NEEDS_FOLDER.has(c.id)) continue
  const run = c.run
  c.run = () => (S.isSpecial(S.pane().path) ? undefined : run())
}

const isSplit = () => (S.state.tabs[S.state.activeTab]?.panes.length ?? 0) > 1
/** Is the command's function key live in the current mode? */
export const fkeyOn = (c: Cmd) => !!c.fkey && (c.fkeyMode === 'split' ? isSplit() : c.fkeyMode === 'single' ? !isSplit() : true)

// Mode-aware display: palette, tooltips and hints read `keys`, so it shows the F-key whenever that is the live binding.
// The non-F-key binding keeps working while hidden (e.g. Ctrl+Shift+N in split mode).
const baseKeys = new Map<Cmd, string | undefined>()
for (const c of commands) {
  if (!c.fkey) continue
  baseKeys.set(c, c.keys)
  Object.defineProperty(c, 'keys', {
    get: () => (fkeyOn(c) && (c.fkeyMode !== undefined || isSplit()) ? c.fkey : baseKeys.get(c)),
    enumerable: true,
  })
}

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
export const matchesCmd = (e: KeyboardEvent, c: Cmd) =>
  (() => {
    const k = baseKeys.has(c) ? baseKeys.get(c) : c.keys
    return !!k && matches(e, k)
  })() || (!!c.alt && matches(e, c.alt)) || (fkeyOn(c) && matches(e, c.fkey!))

/** The mounted drive that contains `path` (longest mount-point prefix), or the path itself. */
function mountRoot(path: string) {
  const norm = (p: string) => p.replace(/\\/g, '/').toLowerCase()
  const target = norm(path)
  const under = (root: string) => target === root || target.startsWith(root.endsWith('/') ? root : `${root}/`)
  const drives = S.state.places.filter((p) => p.total > 0 && under(norm(p.path)))
  return drives.sort((a, b) => b.path.length - a.path.length)[0]?.path ?? path
}

/** Opens the convert dialog for the selected images (non-images are skipped). */
function convertTargets() {
  const paths = S.targets().filter((e) => !e.isDir && isImage(e.path)).map((e) => e.path)
  if (paths.length) openConvert(paths)
  else S.toast({ title: 'Select one or more images to convert', tone: 'info' }, 2500)
}
