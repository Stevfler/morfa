import type { IconName } from './icons'
import { openCleanup } from '../components/cleanup/cleanup'
import { openSettings } from './settings'
import * as S from './store'
import { isConvertible, openConvert } from './convert'
import { t, type MsgKey } from './i18n'
import { canQuickLook, isMac, otherMod, primaryMod, prettyKeys } from './util'

export type Cmd = {
  id: string
  /** Translated; derived from `cmd.<id>`. */
  readonly title: string
  /** Stable group id (`group.<id>` in the dictionary); `groupLabel` is the translated name. */
  group: string
  readonly groupLabel: string
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

export const commands = ([
  { id: 'palette', group: 'general', icon: 'command', keys: 'Ctrl+K', run: () => (S.state.palette = 'all') },
  { id: 'jump', group: 'navigate', icon: 'zap', keys: 'Ctrl+P', run: () => (S.state.palette = 'folders') },
  { id: 'search', group: 'navigate', icon: 'search', keys: 'Ctrl+F', run: () => S.openSearch() },
  { id: 'path', group: 'navigate', icon: 'link', keys: 'Ctrl+L', run: () => (S.state.editPath = true) },
  { id: 'up', group: 'navigate', icon: 'arrow-up', keys: 'Alt+↑', run: () => S.up() },
  { id: 'back', group: 'navigate', icon: 'chevron-left', keys: 'Alt+←', run: () => S.back() },
  { id: 'forward', group: 'navigate', icon: 'chevron-right', keys: 'Alt+→', run: () => S.forward() },
  { id: 'home', group: 'navigate', icon: 'folder-open', keys: 'Alt+Home', run: () => S.go(S.pane(), S.state.home) },
  { id: 'refresh', group: 'navigate', icon: 'refresh', keys: 'Ctrl+R', fkey: 'F5', fkeyMode: 'single', run: () => S.refreshAll() },

  { id: 'newdir', group: 'create', icon: 'folder-plus', keys: 'Ctrl+Shift+N', fkey: 'F7', run: () => S.create('dir') },
  { id: 'newfile', group: 'create', icon: 'file-plus', keys: 'Ctrl+Alt+N', run: () => S.create('file') },

  { id: 'copy', group: 'edit', icon: 'copy', keys: 'Ctrl+C', run: () => S.copyToClipboard('copy') },
  { id: 'cut', group: 'edit', icon: 'scissors', keys: 'Ctrl+X', run: () => S.copyToClipboard('move') },
  { id: 'paste', group: 'edit', icon: 'paste', keys: 'Ctrl+V', run: () => S.paste() },
  { id: 'rename', group: 'edit', icon: 'pencil', keys: 'F2', run: () => S.startRename() },
  { id: 'trash', group: 'edit', icon: 'trash', keys: isMac ? 'Ctrl+Backspace' : 'Del', alt: 'Del', fkey: 'F8', run: () => S.trash() },
  { id: 'delete', group: 'edit', icon: 'x', keys: isMac ? 'Ctrl+Shift+Backspace' : 'Shift+Del', alt: 'Shift+Del', fkey: 'Shift+F8', run: () => S.deleteForever() },
  { id: 'undo', group: 'edit', icon: 'undo', keys: 'Ctrl+Z', run: () => S.undo() },
  { id: 'redo', group: 'edit', icon: 'redo', keys: 'Ctrl+Shift+Z', run: () => S.redo() },
  { id: 'selectall', group: 'edit', icon: 'check-all', keys: 'Ctrl+A', run: () => S.selectAll() },
  { id: 'convert', group: 'edit', icon: 'image', run: () => convertTargets() },
  { id: 'extract', group: 'edit', icon: 'file-archive', run: () => extractTargets() },
  { id: 'compress', group: 'edit', icon: 'file-archive', run: () => S.targets().length && !S.isSpecial(S.pane().path) && S.compress(S.targets().map((e) => e.path), S.pane().path, 'zip') },
  { id: 'copypath', group: 'edit', icon: 'link', keys: 'Ctrl+Shift+C', run: () => S.copyPath() },
  { id: 'properties', group: 'edit', icon: 'info', keys: 'Alt+Enter', run: () => openProperties() },

  { id: 'copyother', group: 'dual', icon: 'copy', fkey: 'F5', fkeyMode: 'split', run: () => S.toOther('copy') },
  { id: 'moveother', group: 'dual', icon: 'arrow-swap', fkey: 'F6', fkeyMode: 'split', run: () => S.toOther('move') },
  { id: 'openapp', group: 'dual', icon: 'external-link', fkey: 'F4', fkeyMode: 'split', run: () => S.openDefault() },
  { id: 'otherpane', group: 'dual', icon: 'columns', run: () => S.focusPane(1 - S.tab().active) },
  { id: 'split', group: 'view', icon: 'columns', keys: 'Ctrl+\\', run: () => S.toggleSplit() },
  // Space is handled as hold-to-preview in keys.ts; from the palette it stays open until Esc/Space.
  { id: 'quicklook', group: 'view', icon: 'quicklook', keys: 'Space', fkey: 'F3', fkeyMode: 'split', run: () => (S.state.quickLook = canQuickLook(S.visible(S.pane())[S.pane().cursor])) },
  { id: 'cleanup', group: 'view', icon: 'broom', run: () => openCleanup(mountRoot(S.pane().path)) },
  { id: 'settings', group: 'view', icon: 'settings', keys: 'Ctrl+,', run: () => openSettings() },
  { id: 'preview', group: 'view', icon: 'panel-right', keys: 'Ctrl+I', run: () => (S.state.preview = !S.state.preview) },
  { id: 'hidden', group: 'view', icon: 'eye', keys: 'Ctrl+H', run: () => (S.state.showHidden = !S.state.showHidden) },
  { id: 'sizes', group: 'view', icon: 'gauge', keys: 'Ctrl+Shift+S', run: () => S.calcFolderSizes() },
  { id: 'newtab', group: 'tabs', icon: 'plus', keys: 'Ctrl+T', run: () => S.newTab() },
  { id: 'closetab', group: 'tabs', icon: 'x', keys: 'Ctrl+W', run: () => S.closeTab() },
  { id: 'bookmark', group: 'folder', icon: 'star', keys: 'Ctrl+D', run: () => S.toggleBookmark() },
  { id: 'terminal', group: 'folder', icon: 'terminal', keys: 'Ctrl+`', run: () => S.openTerminal() },
] as Omit<Cmd, 'title' | 'groupLabel'>[]) as Cmd[]

for (const c of commands) {
  Object.defineProperty(c, 'title', { get: () => t(`cmd.${c.id}` as MsgKey), enumerable: true })
  Object.defineProperty(c, 'groupLabel', { get: () => t(`group.${c.group}` as MsgKey), enumerable: true })
}

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

/** Opens the convert dialog for the selected images/documents (others are skipped). */
function convertTargets() {
  const paths = S.targets().filter((e) => !e.isDir && isConvertible(e.path)).map((e) => e.path)
  if (paths.length) openConvert(paths)
  else S.toast({ title: t('cmd.convertNone'), tone: 'info' }, 2500)
}

/** Unpacks the selection inside an archive, or the selected archive files next to themselves. */
function extractTargets() {
  const p = S.pane()
  if (S.parseArchive(p.path)) return S.extractEntries(p, S.targets())
  const files = S.targets().filter((e) => !e.isDir && S.isArchiveFile(e.name)).map((e) => e.path)
  if (files.length) S.extractArchives(files)
  else S.toast({ title: t('archive.noneSelected'), tone: 'info' }, 2500)
}

/** Properties of the selection, or of the current folder when nothing is selected. */
export function openProperties(p = S.pane()) {
  const t = S.targets(p)
  S.state.properties = t.length ? t.map((e) => e.path) : [p.path]
}
