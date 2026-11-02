import { shallowReactive } from 'vue'
import type { IconPack } from '../settings'
import lucide from './generated/lucide'
export type { IconName } from './map'

/** A pack module: stroke sets get fill/stroke-width from the <svg>; icons are [viewBox, trusted SVG body]. */
export type PackData = { stroke: boolean; icons: Record<string, [string, string]> }

// Lucide is the default, so it ships in the main chunk; every other pack is its own lazy chunk.
const loaders: Record<Exclude<IconPack, 'lucide'>, () => Promise<{ default: PackData }>> = {
  phosphor: () => import('./generated/phosphor'),
  solar: () => import('./generated/solar'),
  fluent: () => import('./generated/fluent'),
  tabler: () => import('./generated/tabler'),
}

export const packs = shallowReactive<Partial<Record<IconPack, PackData>>>({ lucide })
const pending = new Set<IconPack>()

export function loadPack(p: IconPack) {
  if (packs[p] || pending.has(p) || p === 'lucide' || !loaders[p]) return // unknown pack → stays on lucide
  pending.add(p)
  loaders[p]()
    .then((m) => (packs[p] = m.default))
    .catch((e) => console.error(`icon pack ${p} failed to load`, e))
    .finally(() => pending.delete(p))
}

