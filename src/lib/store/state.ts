import { reactive } from 'vue'
import { load } from '../util'
import type { CloudAccount, Entry, Pane, Place, Tab, Toast, Op, UndoItem } from './types'

let uid = 1
export const nextId = () => uid++

export function newPane(path: string): Pane {
  return {
    id: nextId(),
    path,
    back: [],
    fwd: [],
    entries: [],
    loading: false,
    error: null,
    selected: new Set(),
    cursor: 0,
    anchor: 0,
    filter: '',
    sort: { key: 'name', asc: true },
    search: null,
  }
}

export const state = reactive({
  home: '/',
  tabs: [] as Tab[],
  activeTab: 0,
  showHidden: load('morfa.hidden', false),
  preview: load('morfa.preview', false),
  quickLook: false,
  /** Quick Look shows this instead of the pane's cursor, for a dialog's list of files. */
  quickLookFile: null as Entry | null,
  clipboard: null as null | { mode: 'copy' | 'move'; paths: string[] },
  bookmarks: load<string[]>('morfa.bookmarks', []),
  frecency: load<Record<string, { n: number; t: number }>>('morfa.frecency', {}),
  places: [] as Place[],
  cloud: [] as CloudAccount[],
  /** Connect-a-cloud dialog is open. */
  cloudConnect: false,
  sizes: {} as Record<string, number | 'pending'>,
  toasts: [] as Toast[],
  ops: [] as Op[],
  undo: [] as UndoItem[],
  redo: [] as UndoItem[],
  renaming: null as string | null,
  palette: null as null | 'all' | 'folders',
  batchRename: null as null | string[],
  convert: null as null | { paths: string[]; format?: string },
  properties: null as null | string[],
  confirm: null as null | { title: string; desc: string; danger?: boolean; ok: string; run: () => void },
  editPath: false,
})

export const tab = () => state.tabs[state.activeTab]
export const pane = () => tab().panes[tab().active]
export const otherPane = () => (tab().panes.length > 1 ? tab().panes[1 - tab().active] : null)
