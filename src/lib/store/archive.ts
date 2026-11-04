import { invoke } from '@tauri-apps/api/core'
import { basename, parent } from '../util'
import { nextId, state } from './state'
import { fail, toast } from './toast'
import { journalLink, journalRecord, pushUndo } from './history'
import { refreshAll } from './navigation'
import { parseArchive, type Entry, type Pane } from './types'
import { t } from '../i18n'

/** Previews never unpack archive entries bigger than this. */
export const ARCHIVE_PREVIEW_MAX = 50 * 1024 * 1024
export const archiveTooBig = (e: Entry) => !!parseArchive(e.path) && !e.isDir && e.size > ARCHIVE_PREVIEW_MAX

/** The side panel may unpack on its own only where one entry is cheap to reach: not from a compressed tar or 7z. */
export function archiveAutoPreview(e: Entry) {
  const a = parseArchive(e.path)
  return !!a && /\.(zip|tar)$/i.test(a.archive) && !e.isDir && !e.isSymlink && !archiveTooBig(e)
}

const openEntry = (path: string, id: number, preview: boolean) => {
  const a = parseArchive(path)
  return a ? invoke<string>('archive_open', { id, archive: a.archive, rel: a.rel, preview }) : Promise.reject(new Error(`not an archive entry: ${path}`))
}

/** Real path of a file inside an archive, for an explicit open: previews can't cancel it. */
export const archiveLocal = (path: string) => openEntry(path, nextId(), false)

// Previews are latest-wins: one unpack at a time, which Quick Look and the panel share for the same entry.
let preview: { path: string; id: number; result: Promise<string> } | null = null

/** `archiveLocal` for a preview. Rejects if cancelled, by a newer request or `cancelPreviewUnpack`. */
export function previewLocal(path: string) {
  if (preview?.path === path) return preview.result
  cancelPreviewUnpack()
  const id = nextId()
  const result = openEntry(path, id, true).finally(() => {
    if (preview?.id === id) preview = null
  })
  preview = { path, id, result }
  return result
}

export function cancelPreviewUnpack() {
  if (!preview) return
  const { id, result } = preview
  preview = null // a later request for this entry must start over, not join the cancelled one
  // The backend accepts a cancel only once the job runs, which may be after this call: retry until
  // it's accepted or the call has ended.
  let ended = false
  result.catch(() => {}).finally(() => (ended = true))
  const send = async () => {
    const accepted = await invoke<boolean>('archive_cancel', { id }).catch(() => true)
    if (!accepted && !ended) setTimeout(send, 50)
  }
  send()
}

/** Runs one cancellable archive job with a progress row. Resolves to the created paths, or null if it didn't finish. */
async function job(label: string, kind: 'extract' | 'compress', sources: string[], run: (id: number) => Promise<string[]>): Promise<string[] | null> {
  const id = nextId()
  const started = Date.now()
  state.ops.push({ id, label, done: 0, total: 0, current: '', kind: 'archive' })
  try {
    return await run(id)
  } catch (e) {
    const cancelled = String(e).includes('Cancelled')
    journalRecord(kind, label, sources.map((s) => [s, '']), cancelled ? 'cancelled' : 'failed', e, started)
    if (cancelled) toast({ title: t('archive.cancelled'), tone: 'info' })
    else fail(e)
    return null
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

function recordCreated(kind: 'extract' | 'compress', label: string, sources: string[], created: string[], started: number, again: () => Promise<string[]>) {
  let current = created
  // One archive out of many sources, or many items out of one archive.
  const pairs = (): [string, string][] => (kind === 'compress' ? sources.map((s) => [s, current[0]]) : current.map((c) => [sources[0], c]))
  pushUndo({
    label,
    undo: () => invoke('trash_items', { paths: current }),
    redo: async () => {
      current = await again()
    },
    journal: journalLink(kind, label, pairs, 'done', undefined, started),
  })
}

function extract(archive: string, rels: string[], base: string, dest: string, wrap: boolean) {
  const name = basename(archive)
  const call = (id: number) => invoke<string[]>('archive_extract', { id, archive, rels, base, dest, wrap })
  const started = Date.now()
  return job(t('archive.extracting', { name }), 'extract', [archive], call).then((made) => {
    if (made) recordCreated('extract', t('archive.extracted', { name }), [archive], made, started, () => call(nextId()))
  })
}

/** Unpack whole archive files, next to themselves unless `dest` is given. */
export async function extractArchives(files: string[], dest?: string) {
  for (const f of files) await extract(f, [], '', dest ?? parent(f)!, true)
}

/** Unpack items picked inside the archive open in `p`, next to the archive unless `dest` is given. */
export async function extractEntries(p: Pane, entries: Entry[], dest?: string) {
  const a = parseArchive(p.path)
  if (!a || !entries.length) return
  const rels = entries.map((e) => parseArchive(e.path)!.rel)
  await extract(a.archive, rels, a.rel, dest ?? parent(a.archive)!, false)
}

export async function compress(sources: string[], dest: string, format: 'zip' | 'tar.gz') {
  if (!sources.length) return
  const call = async (id: number) => [await invoke<string>('archive_pack', { id, sources, dest, format })]
  const started = Date.now()
  const made = await job(t('archive.compressing', { count: sources.length }), 'compress', sources, call)
  if (made) recordCreated('compress', t('archive.compressed', { name: basename(made[0]) }), sources, made, started, () => call(nextId()))
}
