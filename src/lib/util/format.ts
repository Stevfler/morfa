export function fmtSize(n: number | undefined) {
  if (n == null) return ''
  if (n < 1024) return `${n} B`
  const u = ['KB', 'MB', 'GB', 'TB']
  let i = -1
  do {
    n /= 1024
    i++
  } while (n >= 1024 && i < u.length - 1)
  return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${u[i]}`
}

const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })
const dtf = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' })

/** "5 min ago" for recent things, an absolute date for older ones. */
export function fmtDate(ms: number) {
  if (!ms) return ''
  const diff = (ms - Date.now()) / 1000
  const a = Math.abs(diff)
  if (a < 60) return 'just now'
  if (a < 3600) return rtf.format(Math.round(diff / 60), 'minute')
  if (a < 86400) return rtf.format(Math.round(diff / 3600), 'hour')
  if (a < 86400 * 7) return rtf.format(Math.round(diff / 86400), 'day')
  return dtf.format(ms)
}
export const fmtDateFull = (ms: number) => (ms ? dtf.format(ms) : '')

export const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
