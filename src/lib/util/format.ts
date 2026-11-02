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

const dtf = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' })
const short = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' })

/** Compact date for list columns ("12 Oct 2025"); full date and time live in the info panel. */
export const fmtDate = (ms: number) => (ms ? short.format(ms) : '')
export const fmtDateFull = (ms: number) => (ms ? dtf.format(ms) : '')

export const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
