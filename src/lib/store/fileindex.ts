import { reactive, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { settings } from '../settings'
import { fail } from './toast'

export interface IndexStatus {
  state: 'off' | 'building' | 'ready' | 'cancelled'
  refreshing: boolean
  paused: boolean
  entries: number
  seen: number
  diskBytes: number
  memoryBytes: number
  builtMs: number
  roots: string[]
  skipped: string[]
  watch: 'live' | 'periodic'
  truncated: boolean
}

export const indexStatus = reactive<IndexStatus>({ state: 'off', refreshing: false, paused: false, entries: 0, seen: 0, diskBytes: 0, memoryBytes: 0, builtMs: 0, roots: [], skipped: [], watch: 'live', truncated: false })

// One at a time: the backend must see on and off in the order they happened.
let configuring: Promise<unknown> = Promise.resolve()
const configure = () => {
  configuring = configuring.then(() => invoke('index_configure', { enabled: settings.fileIndex, roots: settings.fileIndexRoots, excludes: settings.fileIndexExcludes })).catch(fail)
}

/** Always tells the backend, off included: that's what removes a file left from an earlier run. */
export async function initFileIndex() {
  await listen<IndexStatus>('index-status', ({ payload }) => void Object.assign(indexStatus, payload))
  configure()
  let timer: ReturnType<typeof setTimeout> | undefined
  // Excludes change per keystroke, and a changed list means a new walk.
  watch(() => [settings.fileIndex, settings.fileIndexRoots.join('\0'), settings.fileIndexExcludes.join('\0')], () => {
    clearTimeout(timer)
    timer = setTimeout(configure, settings.fileIndex ? 800 : 0)
  })
}

export function indexPause(paused: boolean) {
  indexStatus.paused = paused
  invoke('index_pause', { paused }).catch(fail)
}
export const indexCancel = () => invoke('index_cancel').catch(fail)
export const indexRebuild = () => invoke('index_rebuild').catch(fail)
/** Deleted and off: left on, it would quietly come back at the next start. */
export async function indexDelete() {
  settings.fileIndex = false
  await invoke('index_delete').catch(fail)
}
