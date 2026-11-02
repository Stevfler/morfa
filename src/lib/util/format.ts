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
let cached: { loc: string; dtf: Intl.DateTimeFormat; short: Intl.DateTimeFormat } | undefined
const fmts = () => {
  const loc = locale.value
  if (cached?.loc !== loc) cached = { loc, dtf: new Intl.DateTimeFormat(loc, { dateStyle: 'medium', timeStyle: 'short' }), short: new Intl.DateTimeFormat(loc, { day: 'numeric', month: 'short', year: 'numeric' }) }
  return cached
}

/** Compact date for list columns ("12 Oct 2025"); full date and time live in the info panel. */
export const fmtDate = (ms: number) => (ms ? fmts().short.format(ms) : '')
export const fmtDateFull = (ms: number) => (ms ? fmts().dtf.format(ms) : '')

export const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
