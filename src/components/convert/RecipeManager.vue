<script setup lang="ts">
import { computed, ref } from 'vue'
import Icon from '../Icon.vue'
import { addRecipe, deleteRecipe, nameOk, recipeProblem, recipes, recipeSummary, renameRecipe, type ConvertOpts } from '../../lib/convert'
import { toast } from '../../lib/store'
import { t } from '../../lib/i18n'

/** Saves the dialog's options as a recipe; lists saved ones to rename and delete. */
const props = defineProps<{ options: () => ConvertOpts; canSave: boolean }>()
const name = ref('')
const editing = ref<string | null>(null)
const draft = ref('')

const problem = computed(() => recipeProblem(name.value, props.options()))
const saveable = computed(() => props.canSave && !problem.value && !!name.value.trim())

function saveNew() {
  if (!saveable.value) return
  const n = name.value.trim()
  if (addRecipe(n, props.options())) {
    name.value = ''
    toast({ title: t('recipes.saved', { name: n }), tone: 'ok' }, 2500)
  }
}
function edit(id: string, current: string) {
  editing.value = id
  draft.value = current
}
function commit() {
  if (editing.value && renameRecipe(editing.value, draft.value)) editing.value = null
}
</script>

<template>
  <div class="space-y-2">
    <!-- Enter here saves or renames: it must not reach the dialog, where it starts the conversion. -->
    <div class="flex items-center gap-2">
      <input
        v-model="name" spellcheck="false" maxlength="80" :placeholder="t('recipes.namePh')" :aria-label="t('recipes.namePh')"
        class="inp min-w-0 flex-1" @keydown.enter.stop.prevent="saveNew"
      />
      <button type="button" class="h-8 shrink-0 rounded-lg bg-raised/60 px-3 text-[12.5px] text-dim transition hover:bg-raised hover:text-fg disabled:opacity-40" :disabled="!saveable" @click="saveNew">{{ t('recipes.save') }}</button>
    </div>
    <p v-if="problem" class="text-[11.5px] text-danger">{{ t(problem) }}</p>
    <p v-if="!recipes.length" class="text-[11.5px] text-faint">{{ t('recipes.empty') }}</p>
    <ul v-else class="space-y-0.5">
      <li v-for="r in recipes" :key="r.id" class="flex h-9 items-center gap-2 rounded-lg px-2 hover:bg-raised/60">
        <input
          v-if="editing === r.id" v-model="draft" spellcheck="false" maxlength="80" :aria-label="t('recipes.rename')" class="inp min-w-0 flex-1" @vue:mounted="({ el }: { el: HTMLInputElement }) => el.focus()"
          :class="!nameOk(draft, r.id) && 'border-danger'" @keydown.enter.stop.prevent="commit" @keydown.esc.stop.prevent="editing = null" @blur="commit"
        />
        <template v-else>
          <span class="truncate text-[12.5px] text-fg">{{ r.name }}</span>
          <span class="min-w-0 flex-1 truncate text-[11.5px] text-faint" :title="r.outDir ?? t('recipes.nextTo')">{{ recipeSummary(r) }} · {{ r.outDir ?? t('recipes.nextTo') }}</span>
          <button type="button" class="btn" :aria-label="t('recipes.rename')" :title="t('recipes.rename')" @click="edit(r.id, r.name)"><Icon name="pencil" class="size-3.5" /></button>
          <button type="button" class="btn" :aria-label="t('recipes.delete')" :title="t('recipes.delete')" @click="deleteRecipe(r.id)"><Icon name="trash" class="size-3.5" /></button>
        </template>
      </li>
    </ul>
  </div>
</template>

<style scoped>
@reference '../../style.css';
.inp {
  @apply h-8 rounded-md border border-line bg-bg px-2.5 text-[12.5px] text-fg outline-none focus:border-accent/60;
}
.btn {
  @apply grid size-7 shrink-0 place-items-center rounded-md text-faint transition outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60;
}
</style>
