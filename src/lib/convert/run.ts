import { invoke } from '@tauri-apps/api/core'
import { go, nextId, pane, refreshAll, state, toast } from '../store'
import { journalLink, journalRecord } from '../store/history'
import { basename, fmtSize, parent } from '../util'
import { defaultOpts, isAudio, isImage, isVideo, type ConvertOpts } from './formats'
import { t } from '../i18n'
import { canDownload, installOffice, office, refreshOffice } from '../office'

type Result = { pairs: [string, string][]; failed: [string, string][]; cancelled: boolean }

/** "3 images" / "3 videos" / "3 audio files" when every source is that kind, else "3 files". */
function what(sources: string[]) {
  const count = sources.length
  if (sources.every(isImage)) return t('core.images', { count })
  if (sources.every(isVideo)) return t('convert.videos', { count })
  if (sources.every(isAudio)) return t('convert.audios', { count })
  return t('convert.files', { count })
}
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

/** Backend codes in words; anything else is the engine's own message. */
export function reason(err: string) {
  const over = /^OVER_LIMIT:(\d+):(\d+)$/.exec(err)
  if (over) return t('recipes.overLimit', { size: fmtSize(+over[1]), limit: fmtSize(+over[2]) })
  if (err === 'LIMIT_IMAGES_ONLY') return t('recipes.limitImagesOnly')
  if (err === 'BAD_TEMPLATE') return t('recipes.badTemplate')
  if (err === 'NO_OUT_DIR') return t('recipes.noOutDir')
  return err
}

function failed(e: unknown) {
  const msg = String(e)
  if (msg.includes('NO_ENGINE:office')) return noOffice()
  if (msg.includes('NO_ENGINE:ffmpeg'))
    return toast({ title: t('convert.noFfmpeg'), desc: t('convert.noFfmpegDesc'), tone: 'error' }, 12000)
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
  const started = Date.now()
  const fmt = opts.format.toUpperCase()
  state.ops.push({ id, label: t('convert.converting', { what: what(sources), fmt }), done: 0, total: sources.length, current: '', unit: 'files', kind: 'convert' })
  try {
    const r = await call(sources, opts, id)
    if (r.pairs.length) record(r.pairs, opts, fmt, r.cancelled ? 'cancelled' : r.failed.length ? 'partial' : 'done', started, r.failed[0]?.[1])
    else if (r.failed.length || r.cancelled) journalRecord('convert', t('convert.converted', { what: what(sources), fmt }), sources.map((s) => [s, '']), r.cancelled ? 'cancelled' : 'failed', r.failed[0]?.[1], started)
    if (r.failed.length) {
      const [src, err] = r.failed[0]
      if (err.includes('NO_ENGINE:')) failed(err)
      else toast({ title: t('convert.failedN', { count: r.failed.length }), desc: `${basename(src)}: ${reason(err)}`, tone: 'error' }, 10000)
    }
    if (r.cancelled) toast({ title: t('convert.cancelled'), tone: 'info' })
  } catch (e) {
    journalRecord('convert', t('convert.converted', { what: what(sources), fmt }), sources.map((s) => [s, '']), 'failed', e, started)
    failed(e)
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

/** Success toast with "Show", plus an undo entry that trashes the outputs (redo converts again). */
function record(pairs: [string, string][], opts: ConvertOpts, fmt: string, status: 'done' | 'partial' | 'cancelled', started: number, error?: string) {
  // A PDF rendered to images yields one pair per page, so sources repeat.
  let current = pairs
  let outputs = pairs.map(([, d]) => d)
  const sources = [...new Set(pairs.map(([s]) => s))]
  const label = t('convert.converted', { what: what(sources), fmt })
  // Not pushUndo(): its toast offers Undo, ours offers Show (Ctrl+Z still undoes).
  state.undo.push({
    label,
    undo: () => invoke('trash_items', { paths: outputs }),
    redo: async () => {
      const r = await call(sources, opts)
      current = r.pairs
      outputs = r.pairs.map(([, d]) => d)
      if (r.failed.length) throw reason(r.failed[0][1])
    },
    journal: journalLink('convert', label, () => current, status, error, started),
  })
  state.redo = []
  toast({ title: label, tone: 'ok', action: { label: t('convert.show'), run: () => show(outputs[0]) } })
}

export const quickConvert = (paths: string[], format: string) => runConvert(paths, defaultOpts(format))
