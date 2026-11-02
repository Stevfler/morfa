<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription } from 'reka-ui'
import Icon from '../Icon.vue'
import { COLORS, allTags, colorOf, editing, refreshAllTags, setTags, tagsOf } from '../../lib/tags'
import { basename } from '../../lib/util'

const tags = ref<string[]>([])
const draft = ref('')
const open = computed({ get: () => !!editing.value, set: (v) => !v && (editing.value = null) })
const title = computed(() => (editing.value?.length === 1 ? basename(editing.value[0]) : `${editing.value?.length} items`))
// Suggest tags used elsewhere that aren't on these files yet.
const suggestions = computed(() =>
  Object.keys(allTags.value)
    .filter((t) => !tags.value.includes(t) && !colorOf(t) && t.toLowerCase().includes(draft.value.trim().toLowerCase()))
    .slice(0, 8),
)

watch(editing, (paths) => {
  if (!paths) return
  // Start from the tags every selected item shares.
  tags.value = tagsOf(paths[0]).filter((t) => paths.every((p) => tagsOf(p).includes(t)))
  draft.value = ''
  refreshAllTags()
})

function add(t = draft.value) {
  t = t.trim().replace(/,/g, ' ')
  if (t && !tags.value.includes(t)) tags.value.push(t)
  draft.value = ''
}
const toggle = (t: string) => (tags.value.includes(t) ? remove(t) : tags.value.push(t))
const remove = (t: string) => tags.value.splice(tags.value.indexOf(t), 1)

function key(e: KeyboardEvent) {
  if (e.key === 'Enter' || e.key === ',') {
    e.preventDefault()
    if (draft.value.trim()) add()
    else if (e.key === 'Enter') save()
  } else if (e.key === 'Backspace' && !draft.value && tags.value.length) tags.value.pop()
}

async function save() {
  const paths = editing.value
  if (!paths) return
  if (draft.value.trim()) add()
  // Keep per-file tags that weren't shared and so weren't shown for editing.
  const shown = tagsOf(paths[0]).filter((t) => paths.every((p) => tagsOf(p).includes(t)))
  await Promise.all(paths.map((p) => setTags([p], [...tagsOf(p).filter((t) => !shown.includes(t) && !tags.value.includes(t)), ...tags.value])))
  editing.value = null
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-[18vh] left-1/2 z-50 w-[min(420px,92vw)] -translate-x-1/2 rounded-xl bg-panel p-4 ring-1 ring-fg/5 shadow-2xl outline-none">
        <DialogTitle class="truncate text-[13.5px] font-medium">Tags — {{ title }}</DialogTitle>
        <DialogDescription class="pb-3 text-[11.5px] text-faint">Pick colours or type a tag and press Enter</DialogDescription>
        <div class="flex gap-2 pb-3">
          <button
            v-for="c in COLORS" :key="c.name" :title="c.name" :aria-label="c.name" :aria-pressed="tags.includes(c.name)"
            class="grid size-6 place-items-center rounded-full transition hover:scale-110" :style="{ background: c.css }"
            @click="toggle(c.name)"
          >
            <Icon v-if="tags.includes(c.name)" name="check" class="size-3.5 text-white" />
          </button>
        </div>
        <div class="flex min-h-9 flex-wrap items-center gap-1.5 rounded-lg bg-raised px-2 py-1.5 ring-1 ring-fg/5 focus-within:ring-accent">
          <span v-for="t in tags.filter((x) => !colorOf(x))" :key="t" class="flex items-center gap-1 rounded-md bg-accent-soft px-1.5 py-0.5 text-[12px]">
            {{ t }}<Icon name="x" class="size-3 cursor-pointer opacity-60 hover:opacity-100" @click="remove(t)" />
          </span>
          <input v-model="draft" class="min-w-24 flex-1 bg-transparent text-[12.5px] outline-none placeholder:text-faint" placeholder="Add tag…" spellcheck="false" @keydown="key" />
        </div>
        <div v-if="suggestions.length" class="flex flex-wrap gap-1.5 pt-2">
          <button v-for="t in suggestions" :key="t" class="rounded-md px-1.5 py-0.5 text-[11.5px] text-dim ring-1 ring-fg/10 hover:bg-raised" @click="add(t)">+ {{ t }}</button>
        </div>
        <div class="flex justify-end gap-2 pt-4">
          <button class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised" @click="editing = null">Cancel</button>
          <button class="h-8 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent transition hover:brightness-110" @click="save">Save</button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
