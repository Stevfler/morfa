import { ref } from 'vue'
import { fmtSize, load, save } from '../util'
import { isSpecial, toast } from '../store'
import { t, type MsgKey } from '../i18n'
import { caps, loadCaps, targetsFor } from './caps'
import { extOf, FORMATS, formatLabel, gateOpts, isConvertible, isImage, isImageFormat, isLossless, LIMIT_MAX, LIMIT_MIN, validTemplate, type ConvertOpts } from './formats'
import { runConvert } from './run'

/** Saved conversion options. `maxBytes` applies to lossy image output only. */
export type Recipe = Omit<ConvertOpts, 'format'> & { id: string; name: string; format: string }

const KEY = 'morfa.convert.recipes'
const NAME_MAX = 80

const isAbsolute = (p: string) => /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(p)
/** A folder a recipe can write to: on disk, not a pseudo-location or a path relative to nothing. */
const validOutDir = (p: string | null) => p === null || (isAbsolute(p) && !isSpecial(p))

function validResize(r: unknown): r is ConvertOpts['resize'] {
  if (r === null) return true
  const x = r as { mode?: unknown; value?: unknown }
  if (typeof x !== 'object' || !Number.isInteger(x.value)) return false
  const v = x.value as number
  return x.mode === 'percent' ? v >= 1 && v <= 100 : x.mode === 'max' && v >= 16 && v <= 65535
}

/** What's wrong with this recipe, as a message key; null if it's usable. Same rules at save and at load. */
function problem(raw: unknown): MsgKey | null {
  const r = raw as Partial<Recipe> | null
  if (typeof r !== 'object' || !r) return 'recipes.bad'
  const { id, name, format, quality, resize, stripMetadata, maxBytes, outDir, nameTemplate } = r
  if (typeof id !== 'string' || !id || typeof name !== 'string' || !name.trim() || name.length > NAME_MAX) return 'recipes.badName'
  if (typeof format !== 'string' || !FORMATS.some((f) => f.ext === format)) return 'recipes.bad'
  if (!Number.isInteger(quality) || quality! < 1 || quality! > 100) return 'recipes.badQuality'
  if (!validResize(resize)) return 'recipes.badResize'
  if (typeof stripMetadata !== 'boolean') return 'recipes.bad'
  if (maxBytes !== null && (!Number.isSafeInteger(maxBytes) || maxBytes! < LIMIT_MIN || maxBytes! > LIMIT_MAX || !isImageFormat(format) || isLossless(format))) return 'recipes.badLimit'
  if ((outDir !== null && typeof outDir !== 'string') || !validOutDir(outDir ?? null)) return 'recipes.badFolder'
  if (typeof nameTemplate !== 'string' || !validTemplate(nameTemplate)) return 'recipes.badTemplate'
  return null
}

/** Null if the stored value isn't a usable recipe. Never repaired: a recipe that silently lost its limit could write oversized files. */
function parseRecipe(raw: unknown): Recipe | null {
  if (problem(raw)) return null
  const { id, name, format, quality, resize, stripMetadata, maxBytes, outDir, nameTemplate } = raw as Recipe
  return { id, name, format, quality, resize, stripMetadata, maxBytes, outDir, nameTemplate }
}

function parseRecipes(raw: unknown): Recipe[] {
  if (!Array.isArray(raw)) return []
  const ids = new Set<string>()
  const names = new Set<string>()
  return raw.flatMap((x) => {
    const r = parseRecipe(x)
    if (!r || ids.has(r.id) || names.has(folded(r.name))) return []
    ids.add(r.id)
    names.add(folded(r.name))
    return [r]
  })
}

const folded = (name: string) => name.trim().toLowerCase()

export const recipes = ref<Recipe[]>(parseRecipes(load<unknown>(KEY, [])))
const persist = () => save(KEY, recipes.value)

/** Free, sensible name? (Duplicates would be indistinguishable in the menu.) */
export function nameOk(name: string, except?: string) {
  const n = folded(name)
  return !!n && name.trim().length <= NAME_MAX && !recipes.value.some((r) => r.id !== except && folded(r.name) === n)
}

const candidate = (name: string, opts: ConvertOpts): Recipe => ({ ...opts, resize: opts.resize && { ...opts.resize }, id: `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`, name: name.trim() })

/** Why these options can't be saved under this name; null if they can. */
export function recipeProblem(name: string, opts: ConvertOpts): MsgKey | null {
  const p = problem(candidate(name || 'x', opts))
  return p ?? (name.trim() && !nameOk(name) ? 'recipes.nameTaken' : null)
}

export function addRecipe(name: string, opts: ConvertOpts) {
  if (!name.trim() || recipeProblem(name, opts)) return false
  recipes.value.push(candidate(name, opts))
  persist()
  return true
}

export function renameRecipe(id: string, name: string) {
  const r = recipes.value.find((x) => x.id === id)
  if (!r || !nameOk(name, id)) return false
  r.name = name.trim()
  persist()
  return true
}

export function deleteRecipe(id: string) {
  recipes.value = recipes.value.filter((r) => r.id !== id)
  persist()
}

export function recipeSummary(r: Recipe) {
  const size = r.resize && (r.resize.mode === 'percent' ? `${r.resize.value}%` : `${r.resize.value} px`)
  const limit = r.maxBytes && `≤ ${fmtSize(r.maxBytes)}`
  return [formatLabel(r.format), !isLossless(r.format) && `Q${r.quality}`, size, limit].filter(Boolean).join(' · ')
}

const canon = (ext: string) => ({ jpeg: 'jpg', tif: 'tiff', heif: 'heic' })[ext] ?? ext

/** The paths this recipe can convert: real files on a route that exists here. */
export async function fitting(r: Recipe, paths: string[]): Promise<string[]> {
  const local = paths.filter((p) => !isSpecial(p) && isConvertible(p) && (!r.maxBytes || isImage(p)))
  const c = caps.value ?? (await loadCaps().catch(() => null))
  const ok = new Map<string, boolean>()
  await Promise.all([...new Set(local.map(extOf))].map(async (ext) => {
    // Re-encoding an image to its own format works but isn't listed as a target.
    const same = isImage(`x.${ext}`) && canon(ext) === r.format
    ok.set(ext, same ? !!c?.formats.includes(r.format) : (await targetsFor([`x.${ext}`]).catch(() => [] as string[])).includes(r.format))
  }))
  return local.filter((p) => ok.get(extOf(p)))
}

/** Recipes that can take the whole selection, for the context menu. */
export async function recipesFor(paths: string[]): Promise<Recipe[]> {
  const all = await Promise.all(recipes.value.map(async (r) => ((await fitting(r, paths)).length === paths.length ? r : null)))
  return all.filter((r) => !!r)
}

/**
 * Convert what the recipe can take; the rest of the selection (`selected` items, folders included) is
 * left out and said so. Options are gated per file, so a PDF in the selection doesn't cost the images
 * their resize: files needing different options run as separate operations.
 */
export async function runRecipe(r: Recipe, paths: string[], selected = paths.length) {
  const sources = await fitting(r, paths)
  if (!sources.length) return toast({ title: t('recipes.noneFit', { name: r.name }), tone: 'info' }, 4000)
  if (sources.length < selected) toast({ title: t('recipes.skipped', { count: selected - sources.length }), tone: 'info' }, 4000)
  const { id: _id, name: _name, ...opts } = r
  const base: ConvertOpts = { ...opts, resize: opts.resize && { ...opts.resize } }
  const groups = new Map<string, { files: string[]; opts: ConvertOpts }>()
  for (const p of sources) {
    const o = gateOpts([p], base)
    const key = JSON.stringify(o)
    if (groups.has(key)) groups.get(key)!.files.push(p)
    else groups.set(key, { files: [p], opts: o })
  }
  await Promise.all([...groups.values()].map((g) => runConvert(g.files, g.opts)))
}

/** Local files selected in the active pane; empty on pseudo-locations, so recipes aren't offered there. */
export function recipeTargets(entries: { path: string; isDir: boolean }[], panePath: string) {
  return isSpecial(panePath) ? [] : entries.filter((e) => !e.isDir && !isSpecial(e.path)).map((e) => e.path)
}
