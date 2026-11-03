import { reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { openUrl } from '@tauri-apps/plugin-opener'
import { settings } from '../settings'
import { load, save } from '../util'
import { toast } from '../store'
import { t } from '../i18n'

const RELEASES = 'https://github.com/Stevfler/morfa/releases'
const SKIP_KEY = 'morfa.update.skip'

export const upd = reactive({
  /** The card is visible. */
  open: false,
  current: __APP_VERSION__,
  version: '',
  date: '',
  /** Change lines from the release notes, markdown stripped. */
  changes: [] as string[],
  /** 'auto' installs in place; 'manual' (deb/rpm/Arch/macOS) only links to the download page. */
  mode: 'manual' as 'auto' | 'manual',
  phase: 'idle' as 'idle' | 'checking' | 'downloading' | 'installing' | 'error',
  done: 0,
  total: 0,
  error: '',
})

let pending: Update | null = null

/** "- **Fix rename** (abc123)" → "Fix rename"; keeps only the "Changes" bullet list. */
export function parseNotes(notes: string): string[] {
  const out: string[] = []
  for (const line of notes.split('\n')) {
    if (/^##\s/.test(line) && out.length) break
    const m = /^- (.+)$/.exec(line.trim())
    if (!m) continue
    out.push(m[1].replace(/\*\*(.+?)\*\*/g, '$1').replace(/\s*\([0-9a-f]{7,}\)\s*$/, '').trim())
  }
  return out
}

/** Look for a newer release. `manual` = the user asked: report "up to date" and ignore "skip". */
export async function checkUpdates(manual = false) {
  if (!('__TAURI_INTERNALS__' in window) || upd.phase === 'checking' || upd.phase === 'downloading') return
  upd.phase = 'checking'
  try {
    ;[upd.current, upd.mode] = await Promise.all([getVersion(), invoke<'auto' | 'manual'>('update_mode')])
    const u = await check({ timeout: 20000 })
    upd.phase = 'idle'
    if (!u) {
      if (manual) toast({ title: t('update.latest', { version: upd.current }), tone: 'ok' }, 3000)
      return
    }
    if (!manual && load<string>(SKIP_KEY, '') === u.version) return
    pending = u
    Object.assign(upd, {
      version: u.version,
      date: u.date ?? '',
      changes: parseNotes(u.body ?? ''),
      error: '',
      open: true,
    })
  } catch (e) {
    upd.phase = 'idle'
    // A failed background check (offline, no release yet) is not worth a popup.
    if (manual) toast({ title: t('update.checkFailed'), desc: String(e), tone: 'error' })
  }
}

export async function install() {
  if (upd.mode === 'manual' || !pending) return openUrl(`${RELEASES}/tag/v${upd.version}`)
  upd.phase = 'downloading'
  upd.done = 0
  upd.total = 0
  upd.error = ''
  try {
    await pending.downloadAndInstall((ev) => {
      if (ev.event === 'Started') upd.total = ev.data.contentLength ?? 0
      else if (ev.event === 'Progress') upd.done += ev.data.chunkLength
      else if (ev.event === 'Finished') upd.phase = 'installing'
    })
    // Windows: the installer closes the app itself. Elsewhere: restart into the new version.
    await relaunch()
  } catch (e) {
    upd.phase = 'error'
    upd.error = String(e)
  }
}

export function later() {
  upd.open = false
}
export function skip() {
  save(SKIP_KEY, upd.version)
  upd.open = false
}
export const openNotes = () => openUrl(`${RELEASES}/tag/v${upd.version}`)

/** Background check a few seconds after start, so it never competes with the first folder load. */
export function scheduleUpdateCheck() {
  if (settings.checkUpdates) setTimeout(() => checkUpdates(), 4000)
}
