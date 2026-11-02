import type { IconName } from '../icons'

const IMG = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'svg', 'ico', 'heic']
const VID = ['mp4', 'webm', 'mkv', 'mov', 'avi', 'm4v', 'ogv']
const AUD = ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'opus']
const ARC = ['zip', 'tar', 'gz', 'xz', 'zst', '7z', 'rar', 'bz2', 'tgz', 'deb', 'rpm', 'dmg', 'iso']
const CODE = ['js', 'ts', 'tsx', 'jsx', 'vue', 'rs', 'go', 'py', 'rb', 'java', 'kt', 'c', 'h', 'cpp', 'hpp', 'cs', 'swift', 'php', 'sh', 'zsh', 'fish', 'lua', 'json', 'toml', 'yaml', 'yml', 'xml', 'html', 'css', 'scss', 'sql', 'zig', 'nix', 'conf', 'ini', 'lock']
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
export function canQuickLook(e: { isDir: boolean; ext: string; name: string } | undefined) {
  return !!e && ['image', 'text', 'code'].includes(kindOf(e))
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

export const KIND_COLOR: Record<Kind, string> = {
  folder: '#8b6cff',
  image: '#3ecf8e',
  video: '#ff7ab6',
  audio: '#f5a524',
  archive: '#c08457',
  code: '#4fb3ff',
  text: '#a5a9bd',
  pdf: '#ff5c6c',
  file: '#7d8196',
}

export const KIND_LABEL: Record<Kind, string> = {
  folder: 'Folder',
  image: 'Image',
  video: 'Video',
  audio: 'Audio',
  archive: 'Archive',
  code: 'Code',
  text: 'Text',
  pdf: 'PDF',
  file: 'File',
}
