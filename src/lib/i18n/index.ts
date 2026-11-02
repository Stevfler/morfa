import { computed } from 'vue'
import { settings } from '../settings'
import { en, type Messages } from './en'
import { ru } from './ru'

/** Plural forms keyed by Intl.PluralRules categories; `other` is the fallback. */
export type Plural = { one: string; few?: string; many?: string; other: string }
export type Dict = { [K in keyof Messages]: Messages[K] extends string ? string : Plural }
export type MsgKey = keyof Messages
type Params = Record<string, string | number>

/** Adding a language: drop a dictionary here and a label in LANGUAGES. */
const DICTS = { en: en as Dict, ru } satisfies Record<string, Dict>
export type Locale = keyof typeof DICTS
export const LANGUAGES: { id: Locale; label: string }[] = [
  { id: 'en', label: 'English' },
  { id: 'ru', label: 'Русский' },
]

const isLocale = (v: string): v is Locale => v in DICTS
/** First system language we have a dictionary for, else English. */
export function systemLocale(): Locale {
  const prefs = typeof navigator !== 'undefined' ? [...(navigator.languages ?? []), navigator.language] : []
  for (const p of prefs) {
    const base = p?.toLowerCase().split('-')[0]
    if (base && isLocale(base)) return base
  }
  return 'en'
}

export const locale = computed<Locale>(() => (settings.language !== 'system' && isLocale(settings.language) ? settings.language : systemLocale()))
const rules = computed(() => new Intl.PluralRules(locale.value))

const fill = (s: string, p?: Params) => (p ? s.replace(/\{(\w+)\}/g, (m, k) => (k in p ? String(p[k]) : m)) : s)

/** Translate `key`; `{name}` placeholders come from `params`, plural entries pick a form by `params.count`. */
export function t(key: MsgKey, params?: Params): string {
  const v = DICTS[locale.value][key] ?? en[key]
  if (typeof v === 'string') return fill(v, params)
  const pl = v as Plural
  const form = rules.value.select(Number(params?.count ?? 0)) as keyof Plural
  return fill(pl[form] ?? pl.other, params)
}

/** Keep <html lang> in sync for screen readers and hyphenation. */
export function applyLocale() {
  if (typeof document !== 'undefined') document.documentElement.lang = locale.value
}
