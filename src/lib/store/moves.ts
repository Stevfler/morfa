/** Where a file left behind by a failed rollback belongs. Pure. */
const isTemp = (path: string) => path.includes('.morfa-tmp-')

export interface Stranded {
  /** Journal pairs: undoing one moves `at` back to a real name, never onto a temporary one. */
  named: [string, string][]
  /** Files on a temporary name whose real name can't be told. */
  unknown: string[]
}

/** `stranded`: net moves as the backend composed them. Each file is followed from its start through
 *  the steps in order, since in a swap a name is both one file's target and another's source.
 *  `restoreToTarget`: an undo, where the name the file belongs under is where the steps were going;
 *  otherwise it's where the file started. */
export function resolveStranded(steps: [string, string][], stranded: [string, string][], restoreToTarget: boolean): Stranded {
  const out: Stranded = { named: [], unknown: [] }
  for (const [from, at] of stranded) {
    let pos = from
    let seen = false
    for (const [f, t] of steps) {
      if (f !== pos) continue
      pos = t
      seen ||= t === at
    }
    const home = restoreToTarget && pos !== at ? pos : from
    if (seen && !isTemp(home)) out.named.push([home, at])
    else if (isTemp(at)) out.unknown.push(at)
    // On a real name without a known story: the move as reported is the truth, unless its source is temporary.
    else if (!isTemp(from)) out.named.push([from, at])
    else out.unknown.push(at)
  }
  return out
}
