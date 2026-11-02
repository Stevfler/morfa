import { reactive, ref, watch } from 'vue'
import { load, save } from '../util/storage'
import { themeVars, isDark, ACCENTS, THEMES } from './themes'

export type IconPack = 'lucide' | 'phosphor' | 'solar' | 'fluent' | 'tabler'
export type Radius = 'sharp' | 'default' | 'round' | 'extra'
export type Motion = 'full' | 'reduced' | 'off'
export interface Settings {
  theme: string
  accent: string
  radius: Radius
  motion: Motion
  density?: 'compact' | 'comfortable'
  iconPack: IconPack
  respectGitignore: boolean
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
  respectGitignore: false,
}

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
    respectGitignore: typeof r.respectGitignore === 'boolean' ? r.respectGitignore : defaults.respectGitignore,
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
  root.style.colorScheme = isDark(settings.theme) ? 'dark' : 'light'
  root.dataset.theme = settings.theme
  root.dataset.radius = settings.radius
  root.dataset.motion = settings.motion
  root.dataset.density = settings.density ?? 'comfortable'
}

watch(settings, () => {
  applySettings()
  save(KEY, { ...settings })
})
