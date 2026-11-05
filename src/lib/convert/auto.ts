import { reactive, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { nextId, state, toast } from '../store'
import { journalRecord } from '../store/history'
import { settingsOpen } from '../settings'
import { basename } from '../util'
import { t, type MsgKey } from '../i18n'
import { recipes } from './recipes'

export type RuleFailure = { path: string; error: string; at: number }
export type WatchRule = {
  id: string
  folder: string
  recipeId: string
  recipeName: string
  enabled: boolean
  subfolders: boolean
  /** Refusal code: the rule is off and stays off until it's changed. */
  flag: string | null
  nextToSource: boolean
  unavailable: string | null
  noEngine: string | null
  hold: 'mass' | 'rate' | null
  pending: number
  /** Quiet, but cut off by its own format's account: waiting for the rest. */
  incomplete: number
  /** Files that were already there, left alone unless the user opts in. */
  backlog: number
  lastRun: number | null
  done: number
  failed: number
  failures: RuleFailure[]
  running: { file: string; done: number; total: number } | null
}
type Overview = { pausedAll: boolean; standby: boolean; storeError: boolean; ledgerFull: boolean; rules: WatchRule[] }
type Batch = { ruleId: string; recipeName: string; pairs: [string, string][]; failed: [string, string][]; cancelled: boolean; started: number }

export const auto = reactive<Overview>({ pausedAll: false, standby: false, storeError: false, ledgerFull: false, rules: [] })

/** Op-row ids by rule: one row per rule while it converts. */
const opIds = new Map<string, number>()

function apply(o: Overview) {
  auto.pausedAll = o.pausedAll
  auto.standby = o.standby
  auto.storeError = o.storeError
  auto.ledgerFull = o.ledgerFull
  auto.rules = o.rules
  for (const r of o.rules) {
    let op = state.ops.find((x) => x.id === opIds.get(r.id))
    if (r.running) {
      if (!op) {
        const id = nextId()
        opIds.set(r.id, id)
        op = { id, label: '', done: 0, total: 1, current: '', unit: 'files', kind: 'autoconvert' }
        state.ops.push(op)
        op = state.ops[state.ops.length - 1]
      }
      op.label = t('auto.op', { name: r.recipeName, folder: basename(r.folder) })
      op.done = r.running.done
      op.total = r.running.total
      op.current = basename(r.running.file)
    }
  }
  for (const [ruleId, id] of opIds) {
    if (!o.rules.some((r) => r.id === ruleId && r.running)) {
      state.ops = state.ops.filter((x) => x.id !== id)
      opIds.delete(ruleId)
    }
  }
}

const call = (cmd: string, args: Record<string, unknown>) => invoke<Overview>(cmd, args).then(apply)

export const refusal = (code: string) => t(`auto.err.${code}` as MsgKey)

/** Backend refusals are codes; say them, and report the action as not done. */
async function act(cmd: string, args: Record<string, unknown>): Promise<boolean> {
  try {
    await call(cmd, args)
    return true
  } catch (e) {
    toast({ title: t('auto.refused'), desc: refusal(String(e)), tone: 'error' }, 8000)
    return false
  }
}

export const addRule = (folder: string, recipeId: string, subfolders = false) => act('autoconvert_add', { folder, recipeId, subfolders })
export const updateRule = (id: string, change: { recipeId?: string; subfolders?: boolean }) => act('autoconvert_update', { id, ...change })
export const enableRule = (id: string, enabled: boolean) => act('autoconvert_enable', { id, enabled })
export const deleteRule = (id: string) => act('autoconvert_delete', { id })
export const pauseAll = (paused: boolean) => act('autoconvert_pause_all', { paused })
export const resolveHold = (id: string, proceed: boolean) => act('autoconvert_resolve', { id, proceed })
export const processExisting = (id: string) => act('autoconvert_process_existing', { id })
export const clearFailures = (id: string) => act('autoconvert_clear_failures', { id })

/** The op row's cancel. */
export function cancelAutoOp(opId: number) {
  for (const [ruleId, id] of opIds) if (id === opId) return invoke('autoconvert_cancel', { id: ruleId })
}

/** From the folder's context menu: the rule starts off, so say where to turn it on. */
export async function addRuleFromMenu(folder: string, recipeId: string) {
  if (await addRule(folder, recipeId)) toast({ title: t('auto.created'), desc: t('auto.createdDesc'), tone: 'ok', action: { label: t('auto.openSettings'), run: () => (settingsOpen.value = true) } }, 8000)
}

/** At most one summary toast in this window, however many batches end. */
const TOAST_MS = 30_000
let summary = { done: 0, failed: 0 }
let timer: ReturnType<typeof setTimeout> | null = null

function say() {
  timer = null
  const { done, failed } = summary
  summary = { done: 0, failed: 0 }
  if (failed) toast({ title: t('auto.summaryFailed', { count: failed }), desc: t('auto.summaryDesc', { done }), tone: 'error', action: { label: t('auto.openSettings'), run: () => (settingsOpen.value = true) } }, 8000)
  else if (done) toast({ title: t('auto.summary', { count: done }), tone: 'ok' }, 4000)
}

function onBatch(b: Batch) {
  const sources = new Set(b.pairs.map(([s]) => s))
  const pairs: [string, string][] = [...b.pairs, ...b.failed.map(([s]): [string, string] => [s, ''])]
  const status = b.cancelled ? 'cancelled' : !b.pairs.length ? 'failed' : b.failed.length ? 'partial' : 'done'
  if (pairs.length) void journalRecord('convert', t('auto.journal', { name: b.recipeName, count: sources.size + b.failed.length }), pairs, status, b.failed[0]?.[1], b.started)
  summary.done += sources.size
  summary.failed += b.failed.length
  if (!timer) timer = setTimeout(say, summary.failed ? 1000 : TOAST_MS)
}

/** Starts the backend with the saved recipes and keeps it told of changes. */
export async function initAutoConvert() {
  await listen<Overview>('autoconvert-state', ({ payload }) => apply(payload))
  await listen<Batch>('autoconvert-batch', ({ payload }) => onBatch(payload))
  await call('autoconvert_sync', { recipes: recipes.value })
  watch(recipes, (list) => void call('autoconvert_sync', { recipes: list }).catch((e) => console.error('autoconvert', e)), { deep: true })
}
