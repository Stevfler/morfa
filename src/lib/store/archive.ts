import { invoke } from '@tauri-apps/api/core'
import { basename, parent } from '../util'
import { nextId, state } from './state'
import { fail, toast } from './toast'
import { pushUndo } from './history'
import { refreshAll } from './navigation'
import { parseArchive, type Entry, type Pane } from './types'
import { t } from '../i18n'

/** Runs one cancellable archive job with a progress row. Resolves to the created paths, or null if it didn't finish. */
async function job(label: string, run: (id: number) => Promise<string[]>): Promise<string[] | null> {
  const id = nextId()
  state.ops.push({ id, label, done: 0, total: 0, current: '', kind: 'archive' })
  try {
    return await run(id)
  } catch (e) {
    if (String(e).includes('Cancelled')) toast({ title: t('archive.cancelled'), tone: 'info' })
    else fail(e)
    return null
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

function recordCreated(label: string, created: string[], again: () => Promise<string[]>) {
  let current = created
  pushUndo({
    label,
    undo: () => invoke('trash_items', { paths: current }),
    redo: async () => {
      current = await again()
    },
  })
}

function extract(archive: string, rels: string[], base: string, dest: string, wrap: boolean) {
  const name = basename(archive)
  const call = (id: number) => invoke<string[]>('archive_extract', { id, archive, rels, base, dest, wrap })
  return job(t('archive.extracting', { name }), call).then((made) => {
    if (made) recordCreated(t('archive.extracted', { name }), made, () => call(nextId()))
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
  const made = await job(t('archive.compressing', { count: sources.length }), call)
  if (made) recordCreated(t('archive.compressed', { name: basename(made[0]) }), made, () => call(nextId()))
}
