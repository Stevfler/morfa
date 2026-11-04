export type Entry = {
  name: string
  path: string
  isDir: boolean
  isSymlink: boolean
  size: number
  modified: number
  ext: string
  hidden: boolean
  /** Cloud sync mark: 'synced' | 'pending' | 'partial' | '' (server only). Set for cloud views and mirrored folders. */
  cloud?: string
  /** Local mirror path of a cloud entry. */
  local?: string
  /** Online-only cloud document : opened in the browser, never synced. */
  link?: string
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
  /** On a network filesystem; undefined until known, and treated as remote until then. */
  remote?: boolean
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

export type Place = { name: string; path: string; kind: string; total: number; free: number; fs: string; removable: boolean; network: boolean; unavailable: boolean }

/** Backend error for a filesystem call abandoned because the mount stopped answering (see guard.rs). */
export const isNotResponding = (e: unknown) => String(e).startsWith('not-responding')
/** Largest file previewed on a share without being asked: reading more over a slow link is the user's call. */
export const REMOTE_AUTO_MAX = 1 << 20
/** Don't read this file's contents unprompted while the cursor passes over it. */
export const remoteHold = (p: Pane, size: number) => p.remote !== false && size > REMOTE_AUTO_MAX

/** Pseudo-path for the "This PC"-style Drives overview; never sent to the backend. */
export const DRIVES = 'morfa://drives'
export const isSpecial = (path: string) => path.startsWith('morfa://')

/** Remote folders of a cloud account: `morfa://cloud/<id>/<rel>`. Listed via `cloud_list`. */
const CLOUD = 'morfa://cloud/'
export const cloudPath = (id: string, rel = '') => `${CLOUD}${id}${rel ? '/' + rel : ''}`
export function parseCloud(path: string): { id: string; rel: string } | null {
  if (!path.startsWith(CLOUD)) return null
  const rest = path.slice(CLOUD.length)
  const i = rest.indexOf('/')
  return i < 0 ? { id: rest, rel: '' } : { id: rest.slice(0, i), rel: rest.slice(i + 1) }
}

/** Inside an archive file: `morfa://archive/<encoded file path>/<rel>`. Read-only, listed via `archive_list`. */
const ARCHIVE = 'morfa://archive/'
export const archivePath = (archive: string, rel = '') => `${ARCHIVE}${encodeURIComponent(archive)}${rel ? '/' + rel : ''}`
export function parseArchive(path: string): { archive: string; rel: string } | null {
  if (!path.startsWith(ARCHIVE)) return null
  const rest = path.slice(ARCHIVE.length)
  const i = rest.indexOf('/')
  return { archive: decodeURIComponent(i < 0 ? rest : rest.slice(0, i)), rel: i < 0 ? '' : rest.slice(i + 1) }
}
/** Formats the backend can open (src-tauri/src/archive/read.rs). */
export const isArchiveFile = (name: string) => /\.(zip|7z|tar|tgz|tbz2|txz|tzst|tar\.(gz|bz2|xz|zst))$/i.test(name)

/** `host-key-changed: <server> <expected> <presented>` from the SFTP client. */
export function parseHostKeyChange(e: unknown): { server: string; expected: string; got: string } | null {
  const m = /host-key-changed: (\S+) (\S+) (\S+)/.exec(String(e))
  return m ? { server: m[1], expected: m[2], got: m[3] } : null
}

export type CloudStatus = { state: 'idle' | 'syncing' | 'error'; error: string | null; current: string | null; last: number }
export type CloudHold = { pin: string; toServer: boolean; count: number; gone: boolean; confirmed: boolean; sample: string[]; digest: string }
export type CloudAccount = {
  id: string
  kind: 'nextcloud' | 'webdav' | 'sftp'
  name: string
  server: string
  login: string
  localRoot: string
  status: CloudStatus
  pins: string[]
  holds: CloudHold[]
}

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
  /** Progress within the current file (0..1), sent for media conversions. */
  part?: number | null
  /** What done/total count (default 'bytes'). */
  unit?: 'bytes' | 'files'
  kind?: 'transfer' | 'convert' | 'office' | 'archive'
}
