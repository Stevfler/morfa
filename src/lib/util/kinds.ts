import type { IconName } from '../icons'
import { t } from '../i18n'

const IMG = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'svg', 'ico', 'heic']
const VID = ['mp4', 'webm', 'mkv', 'mov', 'avi', 'm4v', 'ogv']
const AUD = ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'opus']
const ARC = ['zip', 'tar', 'gz', 'xz', 'zst', '7z', 'rar', 'bz2', 'tgz', 'deb', 'rpm', 'dmg', 'iso']
const CODE = ['js', 'ts', 'tsx', 'jsx', 'vue', 'rs', 'go', 'py', 'rb', 'java', 'kt', 'c', 'h', 'cpp', 'hpp', 'cs', 'swift', 'php', 'sh', 'zsh', 'fish', 'lua', 'json', 'toml', 'yaml', 'yml', 'xml', 'html', 'css', 'scss', 'sql', 'zig', 'nix', 'conf', 'ini', 'lock']
/** Office files Quick Look reads natively (src-tauri/src/docview.rs). */
export const OFFICE = ['docx', 'docm', 'odt', 'pptx', 'pptm', 'odp', 'xlsx', 'xlsm', 'xlsb', 'xls', 'ods']
const TXT = ['txt', 'md', 'markdown', 'log', 'csv', 'tsv', 'rst', 'org', 'env', 'gitignore']

export type Kind = 'folder' | 'image' | 'video' | 'audio' | 'archive' | 'code' | 'text' | 'pdf' | 'file'

export function kindOf(e: { isDir: boolean; ext: string; name: string }): Kind {
  if (e.isDir) return 'folder'
  const x = e.ext
  if (IMG.includes(x)) return 'image'
  if (VID.includes(x)) return 'video'
  if (AUD.includes(x)) return 'audio'
  if (ARC.includes(x)) return 'archive'
  if (CODE.includes(x)) return 'code'
  if (x === 'pdf') return 'pdf'
  if (TXT.includes(x) || !x || e.name.startsWith('.')) return 'text'
  return 'file'
}

/** Quick Look only handles images and plain text; everything else opens in its own app. */
export function canQuickLook(e: { isDir: boolean; ext: string; name: string; path?: string } | undefined) {
  // Files inside an archive aren't on disk, and the viewers read from disk.
  return !!e && !e.path?.startsWith('morfa://archive/') && (['image', 'text', 'code'].includes(kindOf(e)) || (!e.isDir && OFFICE.includes(e.ext)))
}

export const KIND_ICON: Record<Kind, IconName> = {
  folder: 'folder',
  image: 'file-image',
  video: 'file-video',
  audio: 'file-audio',
  archive: 'file-archive',
  code: 'file-code',
  text: 'file-text',
  pdf: 'file-text',
  file: 'file',
}

/** Resolved via CSS vars (defaults in style.css, overridden by the iconColor setting). */
export const KINDS: Kind[] = ['folder', 'image', 'video', 'audio', 'archive', 'code', 'text', 'pdf', 'file']
export const KIND_COLOR = Object.fromEntries(KINDS.map((k) => [k, `var(--kind-${k})`])) as Record<Kind, string>

/** Translated kind names (reactive getters). */
export const KIND_LABEL = Object.defineProperties({} as Record<Kind, string>, Object.fromEntries(KINDS.map((k) => [k, { get: () => t(`kind.${k}`), enumerable: true }])))
