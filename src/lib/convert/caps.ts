import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { state } from '../store'
import type { Caps, ConvertOpts } from './formats'

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
