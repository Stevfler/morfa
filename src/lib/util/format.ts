import { locale, t } from '../i18n'

export function fmtSize(n: number | undefined) {
  if (n == null) return ''
  if (n < 1024) return `${n} ${t('size.B')}`
  const u = (['size.KB', 'size.MB', 'size.GB', 'size.TB'] as const).map((k) => t(k))
  let i = -1
  do {
    n /= 1024
    i++
  } while (n >= 1024 && i < u.length - 1)
  return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${u[i]}`
}

// Rebuilt when the locale changes; reading `locale` keeps callers reactive.
let cached: { loc: string; rtf: Intl.RelativeTimeFormat; dtf: Intl.DateTimeFormat } | undefined
const fmts = () => {
  const loc = locale.value
  if (cached?.loc !== loc) cached = { loc, rtf: new Intl.RelativeTimeFormat(loc, { numeric: 'auto' }), dtf: new Intl.DateTimeFormat(loc, { dateStyle: 'medium', timeStyle: 'short' }) }
  return cached
}

/** "5 min ago" for recent things, an absolute date for older ones. */
export function fmtDate(ms: number) {
  if (!ms) return ''
  const { rtf, dtf } = fmts()
  const diff = (ms - Date.now()) / 1000
  const a = Math.abs(diff)
  if (a < 60) return t('date.justNow')
  if (a < 3600) return rtf.format(Math.round(diff / 60), 'minute')
  if (a < 86400) return rtf.format(Math.round(diff / 3600), 'hour')
  if (a < 86400 * 7) return rtf.format(Math.round(diff / 86400), 'day')
  return dtf.format(ms)
}
export const fmtDateFull = (ms: number) => (ms ? fmts().dtf.format(ms) : '')

export const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
