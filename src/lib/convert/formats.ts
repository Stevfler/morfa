import { t, type MsgKey } from '../i18n'

export type ConvertOpts = {
  format: string
  quality: number
  resize: null | { mode: 'percent' | 'max'; value: number }
  stripMetadata: boolean
  /** null = next to each source */
  outDir: string | null
  nameTemplate: string
}

export type Caps = { engine: 'vips' | 'magick' | null; formats: string[] }

export const FORMATS = ([
  { ext: 'webp', label: 'WebP', lossy: true },
  { ext: 'avif', label: 'AVIF', lossy: true },
  { ext: 'jpg', label: 'JPEG', lossy: true },
  { ext: 'png', label: 'PNG', lossy: false },
  { ext: 'heic', label: 'HEIC', lossy: true },
  { ext: 'jxl', label: 'JPEG XL', lossy: true },
  { ext: 'tiff', label: 'TIFF', lossy: false },
  { ext: 'gif', label: 'GIF', lossy: false },
  { ext: 'bmp', label: 'BMP', lossy: false },
] as { ext: string; label: string; lossy: boolean; hint: string }[]).map((f) =>
  Object.defineProperty(f, 'hint', { get: () => t(`format.${f.ext}` as MsgKey), enumerable: true }),
)

export const QUICK_FORMATS = ['webp', 'png', 'jpg', 'avif']

export const PRESETS = ([
  { key: 'max', quality: 95 },
  { key: 'balanced', quality: 82 },
  { key: 'compact', quality: 65 },
] as { key: 'max' | 'balanced' | 'compact'; label: string; quality: number }[]).map((p) =>
  Object.defineProperty(p, 'label', { get: () => t(`preset.${p.key}`), enumerable: true }),
)

/** Raster formats vips/magick can read. */
const READABLE = new Set(['jpg', 'jpeg', 'png', 'webp', 'avif', 'heic', 'heif', 'tif', 'tiff', 'gif', 'bmp', 'jxl'])
export const isImage = (path: string) => READABLE.has(path.split(/[\\/]/).pop()!.split('.').pop()!.toLowerCase())

export const defaultOpts = (format: string): ConvertOpts => ({
  format,
  quality: PRESETS[1].quality,
  resize: null,
  stripMetadata: false,
  outDir: null,
  nameTemplate: '{name}',
})
