import { reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Pane } from '../store'
import { fail, startSearch } from '../store'

/** Finder's colour labels; stored as plain tags with these names. */
export const COLORS = [
  { name: 'Red', css: '#ef4444' },
  { name: 'Orange', css: '#f97316' },
  { name: 'Yellow', css: '#eab308' },
  { name: 'Green', css: '#22c55e' },
  { name: 'Blue', css: '#3b82f6' },
  { name: 'Purple', css: '#a855f7' },
  { name: 'Gray', css: '#9ca3af' },
] as const
export const colorOf = (tag: string) => COLORS.find((c) => c.name === tag)?.css

/** path → tags, filled lazily as rows render. */
const cache = reactive(new Map<string, string[]>())
/** tag → number of files, for the sidebar and palette. */
export const allTags = ref<Record<string, number>>({})
/** Paths the Tags… dialog is editing, or null when closed. */
export const editing = ref<string[] | null>(null)

let queue = new Set<string>()
function flush() {
  const paths = [...queue]
  queue = new Set()
  invoke<Record<string, string[]>>('tags_get', { paths })
    // Skip paths setTags wrote while this batch was in flight.
    .then((m) => paths.forEach((p) => cache.has(p) || cache.set(p, m[p] ?? [])))
    .catch(() => paths.forEach((p) => cache.has(p) || cache.set(p, [])))
}

/** Tags of `path`; schedules a batched fetch the first time it is asked. */
export function tagsOf(path: string): string[] {
  const t = cache.get(path)
  if (t) return t
  if (!queue.size) queueMicrotask(flush)
  queue.add(path)
  return []
}

export const refreshAllTags = () => invoke<Record<string, number>>('tags_all').then((t) => (allTags.value = t)).catch(() => {})

export async function setTags(paths: string[], tags: string[]) {
  try {
    await invoke('tags_set', { paths, tags })
    paths.forEach((p) => cache.set(p, tags))
    refreshAllTags()
  } catch (e) {
    fail(e)
  }
}

/** Toggle one tag on every path: add if any lacks it, else remove from all. */
export async function toggleTag(paths: string[], tag: string) {
  // Read fresh so files whose tags haven't loaded yet keep their other tags.
  const cur = await invoke<Record<string, string[]>>('tags_get', { paths }).catch(() => ({}) as Record<string, string[]>)
  const add = paths.some((p) => !(cur[p] ?? []).includes(tag))
  // One call per file, in sequence, since each file keeps its own other tags.
  for (const p of paths) {
    const rest = (cur[p] ?? []).filter((t) => t !== tag)
    await setTags([p], add ? [...rest, tag] : rest)
  }
}

/** Switch the pane to the results view listing every file carrying `tag`. */
export function showTag(p: Pane, tag: string) {
  const query = `#${tag}`
  if (p.search) p.search.query = query
  else p.search = { query, id: -1, results: [], done: false, scanned: 0 }
  return startSearch(p, query).catch(fail)
}
