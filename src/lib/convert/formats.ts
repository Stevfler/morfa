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

export const FORMATS: { ext: string; label: string; lossy: boolean; hint: string }[] = [
  { ext: 'webp', label: 'WebP', lossy: true, hint: 'Best for web' },
  { ext: 'avif', label: 'AVIF', lossy: true, hint: 'Smallest files, newer apps' },
  { ext: 'jpg', label: 'JPEG', lossy: true, hint: 'Photos, works everywhere' },
  { ext: 'png', label: 'PNG', lossy: false, hint: 'Lossless, keeps transparency' },
  { ext: 'heic', label: 'HEIC', lossy: true, hint: 'Apple photos' },
  { ext: 'jxl', label: 'JPEG XL', lossy: true, hint: 'Modern, high quality' },
  { ext: 'tiff', label: 'TIFF', lossy: false, hint: 'Print and archiving' },
  { ext: 'gif', label: 'GIF', lossy: false, hint: 'Simple graphics, 256 colours' },
  { ext: 'bmp', label: 'BMP', lossy: false, hint: 'Uncompressed, legacy apps' },
]

export const QUICK_FORMATS = ['webp', 'png', 'jpg', 'avif']

export const PRESETS: { key: 'max' | 'balanced' | 'compact'; label: string; quality: number }[] = [
  { key: 'max', label: 'Maximum quality', quality: 95 },
  { key: 'balanced', label: 'Balanced', quality: 82 },
  { key: 'compact', label: 'Smallest size', quality: 65 },
]

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
