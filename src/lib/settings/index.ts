import { reactive, ref, watch } from 'vue'
import { load, save } from '../util/storage'
import { themeVars, isDark, ACCENTS, THEMES } from './themes'
import { KINDS } from '../util/kinds'

export type IconPack = 'lucide' | 'phosphor' | 'solar' | 'fluent' | 'tabler'
export type Radius = 'sharp' | 'default' | 'round' | 'extra'
export type IconColor = 'type' | 'accent' | 'mono' | 'custom'
export type Motion = 'full' | 'reduced' | 'off'
export interface Settings {
  theme: string
  accent: string
  radius: Radius
  motion: Motion
  density?: 'compact' | 'comfortable'
  iconPack: IconPack
  iconColor: IconColor
  iconCustom: string
  respectGitignore: boolean
  /** The clean-up tool's expert warning was dismissed with "Don't show again". */
  cleanupWarned: boolean
  /** 'system' follows the OS language. */
  language: string
}

const KEY = 'orbit.settings'
const prefersReduced = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches
const defaults: Settings = {
  theme: 'midnight',
  accent: 'violet',
  radius: 'default',
  motion: prefersReduced ? 'reduced' : 'full',
  density: 'comfortable',
  iconPack: 'lucide',
  iconColor: 'type',
  iconCustom: '#4c8dff',
  respectGitignore: false,
  cleanupWarned: false,
  language: 'system',
}

export const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i
const pick = <T,>(v: unknown, ok: readonly T[], d: T): T => (ok.includes(v as T) ? (v as T) : d)

/** Validate stored settings field by field; unknown keys are dropped, bad values fall back to defaults. */
export function sanitize(raw: unknown): Settings {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  return {
    theme: pick(r.theme, THEMES.map((t) => t.id), defaults.theme),
    accent: pick(r.accent, ACCENTS.map((a) => a.id), defaults.accent),
    radius: pick(r.radius, ['sharp', 'default', 'round', 'extra'] as const, defaults.radius),
    motion: pick(r.motion, ['full', 'reduced', 'off'] as const, defaults.motion),
    density: pick(r.density, ['compact', 'comfortable'] as const, defaults.density!),
    iconPack: pick(r.iconPack, ['lucide', 'phosphor', 'solar', 'fluent', 'tabler'] as const, defaults.iconPack),
    iconColor: pick(r.iconColor, ['type', 'accent', 'mono', 'custom'] as const, defaults.iconColor),
    iconCustom: typeof r.iconCustom === 'string' && HEX.test(r.iconCustom) ? r.iconCustom.toLowerCase() : defaults.iconCustom,
    respectGitignore: typeof r.respectGitignore === 'boolean' ? r.respectGitignore : defaults.respectGitignore,
    cleanupWarned: typeof r.cleanupWarned === 'boolean' ? r.cleanupWarned : defaults.cleanupWarned,
    language: typeof r.language === 'string' && /^(system|[a-z]{2})$/.test(r.language) ? r.language : defaults.language,
  }
}

export const settings = reactive<Settings>(sanitize(load<unknown>(KEY, {})))
export const settingsOpen = ref(false)
export const openSettings = () => void (settingsOpen.value = true)
export { THEMES, ACCENTS } from './themes'

/** Push settings onto <html> as data attributes + CSS vars. Safe to call before mount. */
export function applySettings() {
  const root = document.documentElement
  const vars = themeVars(settings.theme)
  for (const k in vars) root.style.setProperty(k, vars[k])
  const a = ACCENTS.find((x) => x.id === settings.accent) ?? ACCENTS[0]
  root.style.setProperty('--color-accent', a.hex)
  root.style.setProperty('--color-accent-soft', a.hex + '26')
  root.style.setProperty('--color-on-accent', a.on ?? '#ffffff')
  applyIconColors(root, a.hex)
  root.style.colorScheme = isDark(settings.theme) ? 'dark' : 'light'
  root.dataset.theme = settings.theme
  root.dataset.radius = settings.radius
  root.dataset.motion = settings.motion
  root.dataset.density = settings.density ?? 'comfortable'
}

const HAS_MIX = typeof CSS !== 'undefined' && CSS.supports('color', 'color-mix(in oklab, red, blue)')
const rgb = (h: string) => { const x = h.length === 4 ? [...h.slice(1)].map((c) => c + c).join('') : h.slice(1, 7); return [0, 2, 4].map((i) => parseInt(x.slice(i, i + 2), 16)) }
/** sRGB hex mix, used only where color-mix() is unsupported (WebKitGTK < 2.40). */
const mixHex = (a: string, b: string, p: number) => '#' + rgb(a).map((v, i) => Math.round(v * p + rgb(b)[i] * (1 - p)).toString(16).padStart(2, '0')).join('')

/** 'type' falls back to the stylesheet palette; other modes set --kind-* inline. Light themes pull colours toward fg for contrast. */
function applyIconColors(root: HTMLElement, accent: string) {
  const m = settings.iconColor
  const t = themeVars(settings.theme)
  const fg = t['--color-fg'], dim = t['--color-dim']
  const mix = (a: string, b: string, p: number) => (HAS_MIX ? `color-mix(in oklab, ${a} ${p * 100}%, ${b})` : mixHex(a, b, p))
  const base = m === 'accent' ? accent : m === 'custom' ? settings.iconCustom : null
  const legible = (c: string) => (isDark(settings.theme) ? c : mix(c, fg, 0.78))
  for (const k of KINDS) {
    let v: string | null = null
    if (m === 'mono') v = k === 'folder' ? mix(fg, dim, 0.6) : dim
    else if (base) v = legible(k === 'folder' ? base : mix(base, dim, 0.6))
    if (v) root.style.setProperty(`--kind-${k}`, v)
    else root.style.removeProperty(`--kind-${k}`)
  }
}

watch(settings, () => {
  applySettings()
  save(KEY, { ...settings })
})
