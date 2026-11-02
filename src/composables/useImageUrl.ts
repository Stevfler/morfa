import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

// Images go through IPC as blob URLs rather than the asset protocol, which fails on some WebKitGTK setups.
const MIME: Record<string, string> = { svg: 'image/svg+xml' }
const extOf = (path: string) => path.slice(path.lastIndexOf('.') + 1).toLowerCase()
/** Extensions the webview turned out not to decode; those go straight to the backend PNG next time. */
const undecodable = new Set<string>()

async function blobUrl(path: string) {
  const buf = await invoke<ArrayBuffer>('image_bytes', { path })
  const ext = extOf(path)
  return URL.createObjectURL(new Blob([buf], MIME[ext] ? { type: MIME[ext] } : {}))
}

/** Fully decoded before resolving, so showing it never janks. */
const decodes = (url: string) => {
  const img = new Image()
  img.src = url
  return img.decode().then(() => true, () => false)
}

async function load(path: string) {
  const ext = extOf(path)
  if (!undecodable.has(ext)) {
    const url = await blobUrl(path)
    if (await decodes(url)) return url
    URL.revokeObjectURL(url)
  }
  const url = await blobUrl(await invoke<string>('convert_preview', { path }))
  // Remember the extension only once the fallback worked, so one corrupt file doesn't reroute the rest.
  if (await decodes(url)) undecodable.add(ext)
  return url
}

// Small LRU of ready URLs: Quick Look neighbours and the info panel share it, so stepping is instant.
const CACHE = 16
const cache = new Map<string, Promise<string | null>>()

/** Displayable URL for an image file (cached); null if it can't be shown. */
export function imageUrl(path: string) {
  let p = cache.get(path)
  if (p) cache.delete(path)
  else p = load(path).catch(() => (cache.delete(path), null)) // don't keep failures: retry next time
  cache.set(path, p)
  while (cache.size > CACHE) {
    const [old, op] = cache.entries().next().value!
    cache.delete(old)
    op.then((u) => u && URL.revokeObjectURL(u))
  }
  return p
}

/** Reactive `imageUrl` for `path()`: '' while loading, null on failure. */
export function useImageUrl(path: () => string | null | undefined) {
  const url = ref<string | null>('')
  let seq = 0
  watch(path, async (p) => {
    const my = ++seq
    url.value = ''
    if (!p) return
    const u = await imageUrl(p)
    if (my === seq) url.value = u
  }, { immediate: true })
  return url
}
