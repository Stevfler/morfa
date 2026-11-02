import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { state } from '../store'
import { extOf, isImage, isImageFormat, isMedia, isMediaFormat, type Caps, type ConvertOpts } from './formats'

export const caps = ref<Caps | null>(null)
let pending: Promise<Caps> | null = null

/** Engine + saveable formats; asked once per session. */
export function loadCaps(): Promise<Caps> {
  pending ??= invoke<Caps>('convert_caps').then(
    (c) => (caps.value = c),
    (e) => {
      pending = null
      throw e
    },
  )
  return pending
}

export const openConvert = (paths: string[], format?: string) => (state.convert = { paths, format })

export const estimate = (source: string, opts: ConvertOpts) => invoke<{ before: number; after: number }>('convert_estimate', { source, opts })

const targets = new Map<string, Promise<string[]>>()
/** Forget caps and per-extension targets (after an engine is installed or removed). */
export function resetConvertCaches() {
  pending = null
  caps.value = null
  targets.clear()
}
/** Output formats valid for every given file (backend intersection, most useful first); cached per extension set. */
export function targetsFor(paths: string[]): Promise<string[]> {
  const exts = [...new Set(paths.map(extOf))].sort()
  const key = exts.join(',')
  let p = targets.get(key)
  if (!p) {
    p = invoke<string[]>('convert_targets', { exts }).catch((e) => (targets.delete(key), Promise.reject(e)))
    targets.set(key, p)
  }
  return p
}

export type Engine = 'image' | 'office' | 'pandoc' | 'ffmpeg'
/** First engine this route needs that isn't installed (per caps), else null. */
export function missingEngine(paths: string[], format: string, c: Caps | null): Engine | null {
  if (!c) return 'image'
  const exts = paths.map(extOf)
  const media = paths.some(isMedia)
  if ((media || (format !== 'gif' && isMediaFormat(format))) && c.ffmpeg === false) return 'ffmpeg'
  if (media) return null
  const docs = exts.filter((e) => !isImage(`x.${e}`) && e !== 'pdf')
  if (docs.includes('md') && c.pandoc === false) return 'pandoc'
  if (docs.some((e) => e !== 'md') && c.office === false) return 'office'
  const raster = isImageFormat(format) || exts.some((e) => isImage(`x.${e}`))
  return raster && c.engine === null ? 'image' : null
}
