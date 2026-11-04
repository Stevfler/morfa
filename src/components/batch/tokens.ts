/** Tokens of the batch-rename template. Pure: values come in, text comes out, expanded in one pass
 *  so a value that looks like a token ("{n}" as a song title, "$&") stays what it is. */
export interface FileMeta {
  path: string
  modified: number | null
  /** `YYYY-MM-DDTHH:mm:ss`, as written in the file, no time zone. */
  taken: string | null
  camera: string | null
  width: number | null
  height: number | null
  artist: string | null
  album: string | null
  title: string | null
  track: string | null
  year: string | null
}

export type Group = 'stat' | 'image' | 'audio'

const GROUPS: Record<string, Group | null> = {
  name: null, n: null, ext: null,
  date: 'stat', time: 'stat',
  taken: 'image', camera: 'image', width: 'image', height: 'image',
  artist: 'audio', album: 'audio', title: 'audio', track: 'audio', year: 'audio',
}
const FORMATS: Record<string, string> = { date: 'YYYY-MM-DD', time: 'HH-mm-ss', taken: 'YYYY-MM-DD' }

/** Offered as chips, in this order. */
export const CHIPS = ['name', 'n', 'ext', 'date', 'time', 'taken', 'camera', 'width', 'height', 'artist', 'album', 'title', 'track', 'year'] as const

/** `{{abc}}` is the literal text `{abc}`. */
const TOKEN = /\{\{([^{}]*)\}\}|\{([a-zA-Z]+)(?::([^{}]*))?\}/g

export interface Parsed {
  groups: Set<Group>
  /** Tokens nobody knows, or a format on one that takes none: a typo mustn't end up in file names. */
  unknown: string[]
}

export function parseTemplate(template: string): Parsed {
  const groups = new Set<Group>()
  const unknown: string[] = []
  for (const m of template.matchAll(TOKEN)) {
    if (m[2] === undefined) continue
    if (!Object.hasOwn(GROUPS, m[2]) || (m[3] !== undefined && !FORMATS[m[2]])) {
      unknown.push(m[0])
      continue
    }
    const g = GROUPS[m[2]]
    if (g) groups.add(g)
  }
  return { groups, unknown }
}

const p2 = (n: number) => String(n).padStart(2, '0')

export function formatParts(fmt: string, y: number, mo: number, d: number, h: number, mi: number, s: number) {
  const parts: Record<string, string> = { YYYY: String(y).padStart(4, '0'), YY: p2(y % 100), MM: p2(mo), DD: p2(d), HH: p2(h), mm: p2(mi), ss: p2(s) }
  return fmt.replace(/YYYY|YY|MM|DD|HH|mm|ss/g, (k) => parts[k])
}

/** The computer's local time. */
const fromMs = (ms: number, fmt: string) => {
  const d = new Date(ms)
  return formatParts(fmt, d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds())
}

/** Digits as written in the file: going through Date would apply a time zone EXIF doesn't have. */
const fromWritten = (s: string, fmt: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})$/.exec(s)
  return m ? formatParts(fmt, +m[1], +m[2], +m[3], +m[4], +m[5], +m[6]) : null
}

export interface Context {
  stem: string
  /** Without the dot. */
  ext: string
  n: string
  meta: FileMeta | undefined
}

/** `missing`: tokens with no value for this file. They expand to `fallback`, or the row stays as it is. */
export function expand(template: string, c: Context, fallback: string): { text: string; missing: string[] } {
  const missing: string[] = []
  const m = c.meta
  const value = (name: string, fmt: string | undefined): string | null => {
    const f = fmt || FORMATS[name]
    switch (name) {
      case 'name': return c.stem
      case 'n': return c.n
      case 'ext': return c.ext
      case 'date':
      case 'time': return m?.modified != null ? fromMs(m.modified, f) : null
      case 'taken': return m?.taken ? fromWritten(m.taken, f) : null
      case 'width':
      case 'height': return m?.[name] != null ? String(m[name]) : null
      case 'camera':
      case 'artist':
      case 'album':
      case 'title':
      case 'track':
      case 'year': return m?.[name] || null
      default: return null
    }
  }
  const text = template.replace(TOKEN, (whole, literal: string | undefined, name: string, fmt: string | undefined) => {
    if (literal !== undefined) return `{${literal}}`
    if (!Object.hasOwn(GROUPS, name)) return whole
    const v = value(name, fmt)
    if (v !== null) return v
    missing.push(`{${name}}`)
    return fallback
  })
  return { text, missing }
}

/** `{N}` for `{n}`: tokens are lowercase. */
export function lowercaseOf(token: string): string | null {
  const name = /^\{([a-zA-Z]+)/.exec(token)?.[1]
  return name && name !== name.toLowerCase() && Object.hasOwn(GROUPS, name.toLowerCase()) ? `{${name.toLowerCase()}}` : null
}

/** Temporary name for a step of a batch: keeps the original name in sight if there's room on any
 *  disk (eCryptfs allows 143 bytes), the short form near the limit. */
export function tempName(name: string, stamp: string, i: number) {
  const suffix = `.morfa-tmp-${stamp}-${i}`
  return new TextEncoder().encode(name + suffix).length <= 143 ? name + suffix : suffix
}

/** Is the extension placed by the template? Otherwise it's kept automatically at the end. */
export const placesExt = (template: string) => /\.\{ext\}$/.test(template)
