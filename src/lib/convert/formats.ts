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

export type Caps = { engine: 'vips' | 'magick' | null; formats: string[]; office?: boolean; pandoc?: boolean }
export type FormatGroup = 'image' | 'document'

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
  { ext: 'pdf', label: 'PDF', lossy: false, group: 'document' },
  { ext: 'docx', label: 'Word', lossy: false, group: 'document' },
  { ext: 'odt', label: 'ODT', lossy: false, group: 'document' },
  { ext: 'rtf', label: 'RTF', lossy: false, group: 'document' },
  { ext: 'txt', label: 'Text', lossy: true, group: 'document' },
  { ext: 'html', label: 'HTML', lossy: false, group: 'document' },
  { ext: 'epub', label: 'EPUB', lossy: false, group: 'document' },
  { ext: 'xlsx', label: 'Excel', lossy: false, group: 'document' },
  { ext: 'ods', label: 'ODS', lossy: false, group: 'document' },
  { ext: 'csv', label: 'CSV', lossy: true, group: 'document' },
  { ext: 'pptx', label: 'PowerPoint', lossy: false, group: 'document' },
  { ext: 'odp', label: 'ODP', lossy: false, group: 'document' },
] as { ext: string; label: string; lossy: boolean; group?: FormatGroup; hint: string }[]).map((f) =>
  Object.assign(Object.defineProperty(f, 'hint', { get: () => t(`format.${f.ext}` as MsgKey), enumerable: true }), { group: f.group ?? 'image' }) as typeof f & { group: FormatGroup },
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
export const extOf = (path: string) => path.split(/[\\/]/).pop()!.split('.').pop()!.toLowerCase()
export const isImage = (path: string) => READABLE.has(extOf(path))
/** Documents, sheets, slides and PDFs the backend routes through LibreOffice / pandoc / pdf rasterizing. */
const DOCS = new Set(['doc', 'docx', 'odt', 'rtf', 'txt', 'html', 'htm', 'md', 'xls', 'xlsx', 'ods', 'csv', 'ppt', 'pptx', 'odp', 'pdf'])
export const isConvertible = (path: string) => isImage(path) || DOCS.has(extOf(path))
/** True for raster outputs (quality/resize/metadata apply). */
export const isImageFormat = (ext: string) => (FORMATS.find((f) => f.ext === ext)?.group ?? 'image') === 'image'
export const formatLabel = (ext: string) => FORMATS.find((f) => f.ext === ext)?.label ?? ext.toUpperCase()

export const defaultOpts = (format: string): ConvertOpts => ({
  format,
  quality: PRESETS[1].quality,
  resize: null,
  stripMetadata: false,
  outDir: null,
  nameTemplate: '{name}',
})
