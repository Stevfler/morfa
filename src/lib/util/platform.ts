// navigator.platform only: some WebKitGTK builds send a Safari-like "Macintosh" user agent on Linux.
// Values: "MacIntel" (macOS, also on Apple Silicon), "Win32", "Linux x86_64".
const plat = navigator.platform.toLowerCase()

export const isMac = plat.startsWith('mac')
export const isWindows = plat.startsWith('win')

/** The platform's primary shortcut modifier: ⌘ on macOS, Ctrl elsewhere. */
export const primaryMod = (e: KeyboardEvent | MouseEvent) => (isMac ? e.metaKey : e.ctrlKey)
/** The "other" modifier (Ctrl on macOS, Super/Win elsewhere) — never part of our shortcuts. */
export const otherMod = (e: KeyboardEvent | MouseEvent) => (isMac ? e.ctrlKey : e.metaKey)
/** Drag & drop: hold Option (macOS) or Ctrl (elsewhere) to copy instead of move. */
export const copyMod = (e: MouseEvent) => (isMac ? e.altKey : e.ctrlKey)

const MAC_SYM: Record<string, string> = { Ctrl: '⌘', Alt: '⌥', Shift: '⇧', Backspace: '⌫', Del: '⌦', Enter: '↩' }
/** "Ctrl+Shift+N" → ['⌘','⇧','N'] on macOS, ['Ctrl','Shift','N'] elsewhere. */
export function prettyKeys(keys?: string): string[] {
  if (!keys) return []
  const parts = keys.endsWith('++') ? [...keys.slice(0, -2).split('+'), '+'] : keys.split('+')
  return isMac ? parts.map((k) => MAC_SYM[k] ?? k) : parts
}
/** Single-string label, e.g. for menu hints and toast text: "⌘⇧N" / "Ctrl+Shift+N". */
export const keyLabel = (keys: string) => prettyKeys(keys).join(isMac ? '' : '+')
