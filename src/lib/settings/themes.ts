// Palettes: bg/panel/raised/line/fg/dim/faint. fg & dim meet WCAG AA (≥4.5:1) on bg and panel.
export interface Theme { id: string; name: string; dark: boolean; c: [string, string, string, string, string, string, string] }

export const THEMES: Theme[] = [
  { id: 'midnight', name: 'Midnight', dark: true, c: ['#0e0f13', '#15161c', '#1c1d25', '#262833', '#e7e8ee', '#8b8ea0', '#5a5d6e'] },
  { id: 'daylight', name: 'Daylight', dark: false, c: ['#f6f7f9', '#ffffff', '#eef0f4', '#dcdfe6', '#1a1c23', '#5b6070', '#8a8f9e'] },
  { id: 'nord', name: 'Nord', dark: true, c: ['#2b303b', '#2e3440', '#3b4252', '#434c5e', '#eceff4', '#a9b1c2', '#76829a'] },
  { id: 'mocha', name: 'Mocha', dark: true, c: ['#1b1715', '#221d1a', '#2c2522', '#3a322d', '#f1e7dd', '#b3a294', '#7d6f64'] },
  { id: 'oled', name: 'OLED', dark: true, c: ['#000000', '#08080a', '#121215', '#1f1f24', '#ededf0', '#8e8e98', '#5c5c66'] },
  { id: 'forest', name: 'Forest', dark: true, c: ['#101512', '#151c18', '#1c241f', '#27332b', '#e3ece5', '#8fa396', '#5f7066'] },
]

// on: text color on accent fills — dark where white would fail contrast
export const ACCENTS: { id: string; name: string; hex: string; on?: string }[] = [
  { id: 'violet', name: 'Violet', hex: '#8b6cff' },
  { id: 'blue', name: 'Blue', hex: '#4c8dff' },
  { id: 'cyan', name: 'Cyan', hex: '#1fb5c9', on: '#14151a' },
  { id: 'green', name: 'Green', hex: '#2fb36d', on: '#14151a' },
  { id: 'amber', name: 'Amber', hex: '#e09422', on: '#14151a' },
  { id: 'orange', name: 'Orange', hex: '#f0703a', on: '#14151a' },
  { id: 'rose', name: 'Rose', hex: '#ec4f7c' },
  { id: 'slate', name: 'Slate', hex: '#7c86a2', on: '#14151a' },
]

const KEYS = ['bg', 'panel', 'raised', 'line', 'fg', 'dim', 'faint']
export function themeVars(id: string): Record<string, string> {
  const t = THEMES.find((x) => x.id === id) ?? THEMES[0]
  const out: Record<string, string> = {}
  KEYS.forEach((k, i) => (out[`--color-${k}`] = t.c[i]))
  return out
}
export const isDark = (id: string) => (THEMES.find((x) => x.id === id) ?? THEMES[0]).dark
