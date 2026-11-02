export const SEP_RE = /[\\/]/

export function basename(p: string) {
  const parts = p.split(SEP_RE).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

export function sep(p: string) {
  return p.includes('\\') && !p.includes('/') ? '\\' : '/'
}

export function parent(p: string): string | null {
  const s = sep(p)
  const trimmed = p.length > 1 ? p.replace(/[\\/]+$/, '') : p
  const i = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (i < 0 || trimmed === '/' || /^[A-Za-z]:\\?$/.test(trimmed)) return null
  if (i === 0) return '/'
  const head = trimmed.slice(0, i)
  return /^[A-Za-z]:$/.test(head) ? head + s : head
}

export function join(dir: string, name: string) {
  const s = sep(dir)
  return dir.endsWith(s) ? dir + name : dir + s + name
}

/** Breadcrumb segments: [{ label, path }] */
export function crumbs(p: string) {
  const s = sep(p)
  const out: { label: string; path: string }[] = []
  let cur: string | null = p
  while (cur) {
    out.unshift({ label: basename(cur) || cur, path: cur })
    cur = parent(cur)
  }
  if (s === '/' && out[0]?.path === '/') out[0].label = '/'
  return out
}
