<script setup lang="ts">
import { computed, ref } from 'vue'
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent } from 'reka-ui'
import Icon from '../Icon.vue'
import { fmtSize } from '../../lib/util'
import { state } from '../../lib/store'
import { t, type MsgKey } from '../../lib/i18n'
import { en } from '../../lib/i18n/en'
import { cleanup, toggle, emptyTrash, type Category } from './cleanup'

const props = defineProps<{ cat: Category }>()
const open = ref(false)
const isTrash = computed(() => props.cat.id === 'trash')
const free = computed(() => props.cat.items.filter((i) => !i.protected))
const allOn = computed(() => free.value.length > 0 && free.value.every((i) => cleanup.selected.has(i.path)))
const toggleAll = () => {
  const on = !allOn.value
  free.value.forEach((i) => toggle(i, on))
}
const known = (k: string): k is MsgKey => k in en
const label = computed(() => { const k = `cleanup.cat.${props.cat.id}`; return known(k) ? t(k) : props.cat.label })
const desc = computed(() => { const k = `cleanup.cat.${props.cat.id}.desc`; return known(k) ? t(k) : props.cat.desc })
const confirmEmpty = () => {
  state.confirm = {
    title: t('cleanup.emptyTitle'),
    desc: t('cleanup.emptyDesc', { size: fmtSize(props.cat.total) }),
    ok: t('cleanup.emptyOk'), danger: true,
    run: () => void emptyTrash(),
  }
}
const rel = (p: string) => (p.startsWith(cleanup.root) && p !== cleanup.root ? p.slice(cleanup.root.length).replace(/^[\\/]/, '') : p)
</script>

<template>
  <div class="rounded-lg bg-raised/60">
    <div class="flex items-center gap-2 px-3 py-2.5">
      <button
        class="flex min-w-0 flex-1 items-center gap-2 rounded text-left outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
        :aria-expanded="open" :disabled="!cat.items.length" @click="open = !open"
      >
        <Icon name="chevron-right" class="size-3.5 shrink-0 text-dim transition-transform" :class="open && 'rotate-90'" />
        <span class="min-w-0 flex-1">
          <span class="block text-[13px] font-medium">{{ label }}</span>
          <span class="block truncate text-[11.5px] text-dim">{{ desc }}</span>
        </span>
      </button>
      <span class="text-[13px] tabular-nums" :class="cat.total ? 'text-fg' : 'text-dim'">{{ fmtSize(cat.total) }}</span>
      <button
        v-if="isTrash" :disabled="!cat.total || cleanup.busy"
        class="h-7 rounded-full bg-raised px-3 text-[12px] hover:bg-accent-soft disabled:opacity-40" @click="confirmEmpty"
      >{{ t('cleanup.emptyBtn') }}</button>
      <button
        v-else-if="free.length" class="h-7 rounded-md px-2 text-[12px] text-dim hover:bg-raised hover:text-fg"
        @click="toggleAll"
      >{{ allOn ? t('cleanup.none') : t('cleanup.all') }}</button>
    </div>
    <ul v-if="open && !isTrash" class="max-h-72 overflow-y-auto pt-1 pb-2">
      <li v-for="i in cat.items" :key="i.path" class="flex h-7 items-center gap-2 px-3 text-[12px]" :title="i.path">
        <input
          type="checkbox" class="size-3.5 shrink-0 accent-[var(--color-accent)] disabled:opacity-40"
          :checked="cleanup.selected.has(i.path)" :disabled="i.protected" :aria-label="t('cleanup.select', { path: i.path })"
          @change="toggle(i, ($event.target as HTMLInputElement).checked)"
        />
        <TooltipRoot v-if="i.protected">
          <TooltipTrigger as-child>
            <span tabindex="0" class="flex items-center gap-1 rounded bg-danger/15 px-1.5 text-[10.5px] text-danger outline-none focus-visible:ring-2 focus-visible:ring-danger/60">
              <Icon name="lock" class="size-3" /> {{ t('cleanup.system') }}
            </span>
          </TooltipTrigger>
          <TooltipPortal>
            <TooltipContent :side-offset="6" class="z-[60] rounded-md bg-raised px-2 py-1 ring-1 ring-fg/5 text-[12px] shadow-xl">{{ t('cleanup.systemTip') }}</TooltipContent>
          </TooltipPortal>
        </TooltipRoot>
        <span v-else-if="i.risk === 'review'" class="rounded bg-warn/15 px-1.5 text-[10.5px] text-warn">{{ t('cleanup.review') }}</span>
        <span class="min-w-0 flex-1 truncate" :class="i.protected && 'text-dim'">{{ rel(i.path) }}</span>
        <span class="tabular-nums text-dim">{{ fmtSize(i.size) }}</span>
      </li>
    </ul>
  </div>
</template>
