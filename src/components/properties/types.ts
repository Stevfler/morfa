import { fmtDateFull } from '../../lib/util/format'
import { locale, t } from '../../lib/i18n'

export interface Props {
  path: string
  isDir: boolean
  isSymlink: boolean
  linkTarget: string | null
  size: number
  children: number | null
  created: number
  modified: number
  accessed: number
  readonly: boolean
  mode: number | null
  owner: string | null
  group: string | null
  mount: string | null
  fsType: string | null
  device: string | null
  width: number | null
  height: number | null
}

export const fullDate = (ms: number) => fmtDateFull(ms) || '—'
export const bytes = (n: number) => t('props.bytes', { count: n }).replace(String(n), n.toLocaleString(locale.value))

/** 0o755 → "rwxr-xr-x" (with setuid/setgid/sticky letters). */
export function rwx(mode: number) {
  const s = [...'rwxrwxrwx'].map((c, i) => (mode & (1 << (8 - i)) ? c : '-'))
  const special = (i: number, bit: number, ch: string) => {
    if (mode & bit) s[i] = s[i] === 'x' ? ch : ch.toUpperCase()
  }
  special(2, 0o4000, 's')
  special(5, 0o2000, 's')
  special(8, 0o1000, 't')
  return s.join('')
}
export const octal = (mode: number) => mode.toString(8).padStart(4, '0')
