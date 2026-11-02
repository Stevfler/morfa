/** Subsequence fuzzy score; -1 = no match. Rewards consecutive & word-start hits. */
export function fuzzy(query: string, text: string): number {
  if (!query) return 0
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  const direct = t.indexOf(q)
  if (direct >= 0) return 1000 - direct * 2 - (t.length - q.length) * 0.1
  let score = 0
  let ti = 0
  let streak = 0
  for (const ch of q) {
    const found = t.indexOf(ch, ti)
    if (found < 0) return -1
    streak = found === ti ? streak + 1 : 0
    const wordStart = found === 0 || /[\s._\-/\\]/.test(t[found - 1])
    score += 10 + streak * 5 + (wordStart ? 15 : 0) - (found - ti)
    ti = found + 1
  }
  return score
}
