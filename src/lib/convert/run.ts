import { invoke } from '@tauri-apps/api/core'
import { go, nextId, pane, refreshAll, state, toast } from '../store'
import { basename, fmtSize, parent } from '../util'
import { defaultOpts, isImage, type ConvertOpts } from './formats'
import { t } from '../i18n'
import { canDownload, installOffice, office, refreshOffice } from '../office'

type Result = { pairs: [string, string][]; failed: [string, string][]; cancelled: boolean }

/** "3 images" when every source is an image, else "3 files". */
const what = (sources: string[]) => (sources.every(isImage) ? t('core.images', { count: sources.length }) : t('convert.files', { count: sources.length }))
const call = (sources: string[], opts: ConvertOpts, id = nextId()) => invoke<Result>('convert_files', { id, sources, opts })

/** Open the first output's folder in the active pane with the cursor on it. */
function show(path: string) {
  const p = pane()
  const dir = parent(path)!
  if (p.path === dir) {
    p.focusAfterLoad = path
    refreshAll()
  } else go(p, dir, { focus: path })
}

function failed(e: unknown) {
  const msg = String(e)
  if (msg.includes('NO_ENGINE:office')) return noOffice()
  if (msg.includes('NO_ENGINE:pandoc'))
    return toast({ title: t('convert.noPandoc'), desc: t('convert.noPandocDesc'), tone: 'error' }, 12000)
  if (msg.includes('NO_ENGINE:'))
    return toast({ title: t('convert.noEngine'), desc: t('convert.noEngineDesc'), tone: 'error' }, 12000)
  toast({ title: t('convert.failed'), desc: msg, tone: 'error' }, 8000)
}

/** Offer the downloadable module where supported, else the install hint. */
async function noOffice() {
  await refreshOffice()
  if (canDownload())
    return toast({ title: t('office.needed'), desc: t('office.neededDesc', { size: fmtSize(office.value?.downloadSize) }), tone: 'error', action: { label: t('office.download'), run: () => void installOffice() } }, 12000)
  toast({ title: t('convert.noOffice'), desc: t('convert.noOfficeDesc'), tone: 'error' }, 12000)
}

export async function runConvert(sources: string[], opts: ConvertOpts): Promise<void> {
  if (!sources.length) return
  const id = nextId()
  const fmt = opts.format.toUpperCase()
  state.ops.push({ id, label: t('convert.converting', { what: what(sources), fmt }), done: 0, total: sources.length, current: '', unit: 'files', kind: 'convert' })
  try {
    const r = await call(sources, opts, id)
    if (r.pairs.length) record(r.pairs, opts, fmt)
    if (r.failed.length) {
      const [src, err] = r.failed[0]
      if (err.includes('NO_ENGINE:')) failed(err)
      else toast({ title: t('convert.failedN', { count: r.failed.length }), desc: `${basename(src)}: ${err}`, tone: 'error' }, 10000)
    }
    if (r.cancelled) toast({ title: t('convert.cancelled'), tone: 'info' })
  } catch (e) {
    failed(e)
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

/** Success toast with "Show", plus an undo entry that trashes the outputs (redo converts again). */
function record(pairs: [string, string][], opts: ConvertOpts, fmt: string) {
  // A PDF rendered to images yields one pair per page, so sources repeat.
  let outputs = pairs.map(([, d]) => d)
  const sources = [...new Set(pairs.map(([s]) => s))]
  const label = t('convert.converted', { what: what(sources), fmt })
  // Not pushUndo(): its toast offers Undo, ours offers Show (Ctrl+Z still undoes).
  state.undo.push({
    label,
    undo: () => invoke('trash_items', { paths: outputs }),
    redo: async () => {
      const r = await call(sources, opts)
      outputs = r.pairs.map(([, d]) => d)
      if (r.failed.length) throw r.failed[0][1]
    },
  })
  state.redo = []
  toast({ title: label, tone: 'ok', action: { label: t('convert.show'), run: () => show(outputs[0]) } })
}

export const quickConvert = (paths: string[], format: string) => runConvert(paths, defaultOpts(format))
