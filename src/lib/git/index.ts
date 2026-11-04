import { reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { isSpecial } from '../store/types'

/** folder → branch, or null when it isn't a repository's root. */
const cache = reactive(new Map<string, string | null>())
/** Which generation each folder was last asked in. Bumping it rechecks the folders on screen only. */
const asked = new Map<string, number>()
const generation = ref(0)

let queue = new Set<string>()
function flush() {
  const paths = [...queue]
  queue = new Set()
  invoke<Record<string, string>>('git_branches', { paths })
    .then((m) => paths.forEach((p) => cache.set(p, m[p] ?? null)))
    .catch(() => {})
}

/** Branch of the repository rooted at `path`; schedules a batched check when it's due one. */
export function branchOf(path: string): string | null {
  const gen = generation.value
  // Archive and cloud paths are all morfa://.
  if (asked.get(path) !== gen && !isSpecial(path)) {
    asked.set(path, gen)
    if (!queue.size) queueMicrotask(flush)
    queue.add(path)
  }
  return cache.get(path) ?? null
}

/** A branch switch happens outside the app and in a folder nothing watches: check again now and then. */
export function recheckBranches() {
  generation.value++
}
