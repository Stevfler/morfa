import { invoke } from '@tauri-apps/api/core'
import { go, nextId, pane, refreshAll, state, toast } from '../store'
import { basename, parent } from '../util'
import { defaultOpts, type ConvertOpts } from './formats'

type Result = { pairs: [string, string][]; failed: [string, string][]; cancelled: boolean }

const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`
const call = (sources: string[], opts: ConvertOpts, id = nextId()) => invoke<Result>('convert_images', { id, sources, opts })

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
  if (msg.includes('NO_ENGINE:'))
    return toast({ title: 'No image converter found', desc: 'Install libvips (e.g. sudo pacman -S libvips) or ImageMagick, then try again.', tone: 'error' }, 12000)
  toast({ title: 'Conversion failed', desc: msg, tone: 'error' }, 8000)
}

export async function runConvert(sources: string[], opts: ConvertOpts): Promise<void> {
  if (!sources.length) return
  const id = nextId()
  const fmt = opts.format.toUpperCase()
  state.ops.push({ id, label: `Converting ${plural(sources.length, 'image')} → ${fmt}`, done: 0, total: sources.length, current: '', unit: 'files', kind: 'convert' })
  try {
    const r = await call(sources, opts, id)
    if (r.pairs.length) record(r.pairs, opts, fmt)
    if (r.failed.length) {
      const [src, err] = r.failed[0]
      toast({ title: `${plural(r.failed.length, 'image')} couldn’t be converted`, desc: `${basename(src)}: ${err}`, tone: 'error' }, 10000)
    }
    if (r.cancelled) toast({ title: 'Conversion cancelled', tone: 'info' })
  } catch (e) {
    failed(e)
  } finally {
    state.ops = state.ops.filter((o) => o.id !== id)
    refreshAll()
  }
}

/** Success toast with "Show", plus an undo entry that trashes the outputs (redo converts again). */
function record(pairs: [string, string][], opts: ConvertOpts, fmt: string) {
  let outputs = pairs.map(([, d]) => d)
  const sources = pairs.map(([s]) => s)
  const label = `Converted ${plural(pairs.length, 'image')} to ${fmt}`
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
  toast({ title: label, tone: 'ok', action: { label: 'Show', run: () => show(outputs[0]) } })
}

export const quickConvert = (paths: string[], format: string) => runConvert(paths, defaultOpts(format))
