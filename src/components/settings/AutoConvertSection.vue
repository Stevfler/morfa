<script setup lang="ts">
import { ref } from 'vue'
import { open as pickPath } from '@tauri-apps/plugin-dialog'
import { addRule, auto, clearFailures, deleteRule, enableRule, pauseAll, processExisting, recipes, refusal, resolveHold, updateRule, type WatchRule } from '../../lib/convert'
import { reason } from '../../lib/convert/run'
import { fmtDateFull, basename } from '../../lib/util'
import { t, type MsgKey } from '../../lib/i18n'
import Icon from '../Icon.vue'
import Row from './Row.vue'
import Toggle from './Toggle.vue'

const recipeId = ref('')
const sub = ref(false)
const button = 'h-7 rounded-md bg-raised px-3 text-[12px] text-fg hover:bg-raised/70 disabled:opacity-40'
const input = 'h-7 min-w-0 rounded-md bg-raised px-2 text-[12px] text-fg outline-none'
const open = ref<string | null>(null)

async function add() {
  const dir = await pickPath({ directory: true, multiple: false })
  if (typeof dir === 'string' && recipeId.value) await addRule(dir, recipeId.value, sub.value)
}

function status(r: WatchRule): string {
  if (r.flag) return refusal(r.flag)
  if (!r.enabled) return t('auto.state.off')
  if (r.unavailable) return r.unavailable === 'outputMissing' ? t('auto.state.noOutput') : r.unavailable === 'tooMany' ? t('auto.state.tooMany') : t('auto.state.unavailable')
  if (auto.pausedAll) return t('auto.state.paused')
  if (r.hold) return t(`auto.hold.${r.hold}` as MsgKey, { count: r.pending })
  if (r.running || r.pending) return t('auto.state.working', { count: r.pending })
  return t('auto.state.watching')
}
const counts = (r: WatchRule) => [t('auto.done', { count: r.done }), r.failed && t('auto.failed', { count: r.failed }), r.lastRun && t('auto.lastRun', { when: fmtDateFull(r.lastRun) })].filter(Boolean).join(' · ')
</script>

<template>
  <section class="flex flex-col gap-1">
    <p class="py-1 text-[12px] text-fg">{{ t('auto.onlyOpen') }}</p>
    <p class="pb-1 text-[11.5px] text-dim">{{ t('auto.intro') }}</p>
    <p v-if="auto.standby" class="py-1 text-[12px] text-danger">{{ t('auto.standby') }}</p>
    <p v-if="auto.ledgerFull" class="pb-1 text-[11.5px] text-dim">{{ t('auto.ledgerFull') }}</p>
    <Row :title="t('auto.pauseAll')" :desc="t('auto.pauseAllDesc')">
      <Toggle :model-value="auto.pausedAll" :label="t('auto.pauseAll')" @update:model-value="pauseAll($event)" />
    </Row>

    <div class="flex flex-wrap items-center gap-2 py-2">
      <select v-model="recipeId" :class="input" class="flex-1" :aria-label="t('auto.recipe')">
        <option value="" disabled>{{ recipes.length ? t('auto.pickRecipe') : t('auto.noRecipes') }}</option>
        <option v-for="x in recipes" :key="x.id" :value="x.id">{{ x.name }}</option>
      </select>
      <label class="flex items-center gap-1.5 text-[12px] text-dim"><input v-model="sub" type="checkbox"> {{ t('auto.subfolders') }}</label>
      <button :class="button" :disabled="!recipeId" @click="add">{{ t('auto.add') }}</button>
    </div>

    <p v-if="!auto.rules.length" class="py-2 text-[11.5px] text-faint">{{ t('auto.empty') }}</p>
    <ul class="flex flex-col gap-2">
      <li v-for="r in auto.rules" :key="r.id" class="rounded-lg bg-raised/40 p-2.5">
        <div class="flex items-center gap-2">
          <div class="min-w-0 flex-1">
            <div class="truncate text-[12.5px] text-fg" :title="r.folder">{{ basename(r.folder) }} → {{ r.recipeName }}</div>
            <div class="truncate text-[11.5px]" :class="r.flag || r.hold || r.unavailable ? 'text-danger' : 'text-dim'" :title="r.folder">{{ status(r) }}</div>
          </div>
          <Toggle :model-value="r.enabled" :label="t('auto.enable')" @update:model-value="enableRule(r.id, $event)" />
          <button class="grid size-7 place-items-center rounded-md text-dim hover:bg-raised hover:text-fg" :aria-label="t('auto.delete')" :title="t('auto.delete')" @click="deleteRule(r.id)"><Icon name="trash" class="size-3.5" /></button>
        </div>
        <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-dim">
          <span v-if="r.done || r.failed || r.lastRun">{{ counts(r) }}</span>
          <label class="flex items-center gap-1.5"><input type="checkbox" :checked="r.subfolders" :disabled="r.flag === 'recipeMissing'" @change="updateRule(r.id, { subfolders: ($event.target as HTMLInputElement).checked })"> {{ t('auto.subfolders') }}</label>
          <span v-if="r.nextToSource">{{ t('auto.nextToSource') }}</span>
        </div>
        <div v-if="r.flag === 'recipeMissing'" class="mt-1.5 flex items-center gap-2">
          <select :class="input" class="flex-1" :aria-label="t('auto.recipe')" :value="''" @change="updateRule(r.id, { recipeId: ($event.target as HTMLSelectElement).value })">
            <option value="" disabled>{{ t('auto.pickRecipe') }}</option>
            <option v-for="x in recipes" :key="x.id" :value="x.id">{{ x.name }}</option>
          </select>
        </div>
        <div v-if="r.incomplete && r.enabled" class="mt-1 text-[11.5px] text-dim">{{ t('auto.incomplete', { count: r.incomplete }) }}</div>
        <div v-if="r.noEngine && r.enabled" class="mt-1 text-[11.5px] text-danger">{{ t('auto.noEngine') }}</div>
        <div v-if="r.hold" class="mt-1.5 flex gap-1.5">
          <button :class="button" @click="resolveHold(r.id, true)">{{ t('auto.continue', { count: r.pending }) }}</button>
          <button :class="button" @click="resolveHold(r.id, false)">{{ t('auto.leave') }}</button>
        </div>
        <div v-if="r.enabled && r.backlog && !r.hold" class="mt-1.5 flex items-center gap-2 text-[11.5px] text-dim">
          <span class="min-w-0 flex-1">{{ t('auto.backlog', { count: r.backlog }) }}</span>
          <button :class="button" @click="processExisting(r.id)">{{ t('auto.processExisting', { count: r.backlog }) }}</button>
        </div>
        <div v-if="r.failures.length" class="mt-1.5 text-[11.5px]">
          <button class="text-dim underline-offset-2 hover:underline" @click="open = open === r.id ? null : r.id">{{ t('auto.showFailures', { count: r.failures.length }) }}</button>
          <button class="ml-3 text-dim underline-offset-2 hover:underline" @click="clearFailures(r.id)">{{ t('auto.clearFailures') }}</button>
          <ul v-if="open === r.id" class="mt-1 flex max-h-40 flex-col gap-0.5 overflow-y-auto">
            <li v-for="f in r.failures" :key="f.path + f.at" class="truncate text-danger" :title="`${f.path}\n${f.error}`">{{ basename(f.path) }}: {{ f.error === 'CANT_OPEN' ? t('auto.cantOpen') : reason(f.error) }}</li>
          </ul>
        </div>
      </li>
    </ul>
  </section>
</template>
