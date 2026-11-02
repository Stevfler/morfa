export type Entry = {
  name: string
  path: string
  isDir: boolean
  isSymlink: boolean
  size: number
  modified: number
  ext: string
  hidden: boolean
}

export type SortKey = 'name' | 'size' | 'modified' | 'ext'

export type Pane = {
  id: number
  path: string
  back: string[]
  fwd: string[]
  entries: Entry[]
  loading: boolean
  error: string | null
  selected: Set<string>
  cursor: number
  anchor: number
  filter: string
  sort: { key: SortKey; asc: boolean }
  search: null | { query: string; id: number; results: Entry[]; done: boolean; scanned: number }
  /** Restore cursor to this path after the next load (e.g. when going up a level). */
  focusAfterLoad?: string
}

export type Tab = { id: number; panes: Pane[]; active: number }

export type Place = { name: string; path: string; kind: string; total: number; free: number; fs: string; removable: boolean }

/** Pseudo-path for the "This PC"-style Drives overview; never sent to the backend. */
export const DRIVES = 'orbit://drives'
export const isSpecial = (path: string) => path.startsWith('orbit://')

/** A volume that is not mounted yet (from `list_drives`). */
export type Drive = { name: string; device: string; fstype: string; size: number; encrypted: boolean; removable: boolean }

export type UndoItem = { label: string; undo: () => Promise<unknown>; redo: () => Promise<unknown> }

export type Toast = {
  id: number
  title: string
  desc?: string
  tone?: 'ok' | 'error' | 'info'
  action?: { label: string; run: () => void }
}

export type Op = {
  id: number
  label: string
  done: number
  total: number
  current: string
  /** What done/total count (default 'bytes'). */
  unit?: 'bytes' | 'files'
  kind?: 'transfer' | 'convert'
}
