import { t, type MsgKey } from '../i18n'

export type ConvertOpts = {
  format: string
  quality: number
  resize: null | { mode: 'percent' | 'max'; value: number }
  stripMetadata: boolean
  /** null = next to each source */
  outDir: string | null
  nameTemplate: string
  /** Per-file size limit (lossy image output from image sources); null = none */
  maxBytes: number | null
}

export type Caps = { engine: 'vips' | 'magick' | null; formats: string[]; office?: boolean; pandoc?: boolean; ffmpeg?: boolean }
export type FormatGroup = 'image' | 'document' | 'video' | 'audio'

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
  { ext: 'mp4', label: 'MP4', lossy: true, group: 'video' },
  { ext: 'webm', label: 'WebM', lossy: true, group: 'video' },
  { ext: 'mkv', label: 'MKV', lossy: true, group: 'video' },
  { ext: 'mov', label: 'MOV', lossy: true, group: 'video' },
  { ext: 'mp3', label: 'MP3', lossy: true, group: 'audio' },
  { ext: 'm4a', label: 'M4A (AAC)', lossy: true, group: 'audio' },
  { ext: 'ogg', label: 'OGG Vorbis', lossy: true, group: 'audio' },
  { ext: 'opus', label: 'Opus', lossy: true, group: 'audio' },
  { ext: 'flac', label: 'FLAC', lossy: false, group: 'audio' },
  { ext: 'wav', label: 'WAV', lossy: false, group: 'audio' },
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
const VIDEO = new Set(['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v', 'wmv', 'flv', 'mpg', 'mpeg', '3gp'])
const AUDIO = new Set(['mp3', 'wav', 'flac', 'ogg', 'oga', 'm4a', 'aac', 'opus', 'wma'])
/** Video / audio sources the backend routes through FFmpeg. */
export const isVideo = (path: string) => VIDEO.has(extOf(path))
export const isAudio = (path: string) => AUDIO.has(extOf(path))
export const isMedia = (path: string) => isVideo(path) || isAudio(path)
export const isConvertible = (path: string) => isImage(path) || DOCS.has(extOf(path)) || isMedia(path)
/** True for raster outputs (quality/resize/metadata apply). */
export const isImageFormat = (ext: string) => (FORMATS.find((f) => f.ext === ext)?.group ?? 'image') === 'image'
export const formatGroup = (ext: string): FormatGroup => FORMATS.find((f) => f.ext === ext)?.group ?? 'image'
/** Video/audio output; gif counts as video when made from a media source. */
export const isMediaFormat = (ext: string, fromMedia = false) => ['video', 'audio'].includes(formatGroup(ext)) || (fromMedia && ext === 'gif')
export const isLossless = (ext: string) => FORMATS.find((f) => f.ext === ext)?.lossy === false
export const formatLabel = (ext: string) => FORMATS.find((f) => f.ext === ext)?.label ?? ext.toUpperCase()

export const defaultOpts = (format: string): ConvertOpts => ({
  format,
  quality: PRESETS[1].quality,
  resize: null,
  stripMetadata: false,
  outDir: null,
  nameTemplate: '{name}',
  maxBytes: null,
})

export const LIMIT_MIN = 1024
export const LIMIT_MAX = 10 * 1024 ** 3
const LIMIT_UNITS: Record<string, number> = { kb: 1024, 'кб': 1024, mb: 1024 ** 2, 'мб': 1024 ** 2, gb: 1024 ** 3, 'гб': 1024 ** 3 }

/**
 * Size limit as typed: megabytes unless a unit says otherwise. '' is no limit (null); anything not
 * clearly one size is 'invalid', including `1,000` and `1 000`, which could mean 1 or 1000.
 */
export function parseLimit(text: string): number | null | 'invalid' {
  const s = text.trim().toLowerCase()
  if (!s) return null
  const m = /^(\d+)(?:[.,](\d+))?\s*(kb|mb|gb|кб|мб|гб)?$/.exec(s)
  if (!m || (m[2]?.length === 3 && !/^0+$/.test(m[1]))) return 'invalid'
  const bytes = Math.round(Number(`${m[1]}.${m[2] ?? 0}`) * LIMIT_UNITS[m[3] ?? 'mb'])
  return bytes >= LIMIT_MIN && bytes <= LIMIT_MAX ? bytes : 'invalid'
}

/** A name template that can't leave the output folder (`D:evil` on Windows). */
export const validTemplate = (s: string) => !!s.trim() && !s.includes(':')

/** Which options apply to these sources and format; shared by the dialog and recipes. */
export function applicable(paths: string[], format: string) {
  const mediaOut = isMediaFormat(format, paths.some(isMedia))
  const audioOut = formatGroup(format) === 'audio'
  const imageOut = isImageFormat(format) && !mediaOut
  const imageIn = imageOut && paths.every(isImage)
  const videoResize = mediaOut && !audioOut && paths.every((p) => isVideo(p) || extOf(p) === 'gif')
  return { mediaOut, audioOut, imageOut, imageIn, videoResize, resize: imageIn || videoResize, strip: imageIn || mediaOut, limit: imageIn && !isLossless(format) }
}

/** The options with everything that doesn't apply to these sources switched off. */
export function gateOpts(paths: string[], o: ConvertOpts): ConvertOpts {
  const a = applicable(paths, o.format)
  return { ...o, resize: a.resize ? o.resize : null, stripMetadata: a.strip && o.stripMetadata, maxBytes: a.limit ? o.maxBytes : null }
}
