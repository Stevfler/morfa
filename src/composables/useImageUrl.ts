import { onBeforeUnmount, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

// Images go through IPC as blob URLs rather than the asset protocol, which fails on some WebKitGTK setups.
const MIME: Record<string, string> = { svg: 'image/svg+xml' }

async function blobUrl(path: string) {
  const buf = await invoke<ArrayBuffer>('image_bytes', { path })
  const ext = path.slice(path.lastIndexOf('.') + 1).toLowerCase()
  return URL.createObjectURL(new Blob([buf], MIME[ext] ? { type: MIME[ext] } : {}))
}

const decodes = (url: string) =>
  new Promise<boolean>((res) => {
    const img = new Image()
    img.onload = () => res(true)
    img.onerror = () => res(false)
    img.src = url
  })

/** Displayable URL for an image file; formats the webview can't decode (AVIF/HEIC/TIFF…) get a backend-rendered PNG. */
async function imageUrl(path: string) {
  const url = await blobUrl(path)
  if (await decodes(url)) return url
  URL.revokeObjectURL(url)
  return blobUrl(await invoke<string>('convert_preview', { path }))
}

/** Reactive `imageUrl` for `path()`; '' while loading, null on failure. Revokes stale URLs. */
export function useImageUrl(path: () => string | null | undefined) {
  const url = ref<string | null>('')
  let seq = 0
  const drop = () => url.value && URL.revokeObjectURL(url.value)
  watch(path, async (p) => {
    const my = ++seq
    drop()
    url.value = ''
    if (!p) return
    const u = await imageUrl(p).catch(() => null)
    if (my === seq) url.value = u
    else if (u) URL.revokeObjectURL(u)
  }, { immediate: true })
  onBeforeUnmount(drop)
  return url
}
