<script setup lang="ts">
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import logo from '../../src-tauri/icons/128x128@2x.png'
import { upd, install, later, skip, openNotes } from '../lib/update'
import { fmtSize } from '../lib/util'
import { t } from '../lib/i18n'

const expanded = ref(false)
const busy = computed(() => upd.phase === 'downloading' || upd.phase === 'installing')
const shown = computed(() => (expanded.value ? upd.changes : upd.changes.slice(0, 3)))
const pct = computed(() => (upd.total ? Math.min(100, (upd.done / upd.total) * 100) : 0))
const date = computed(() => {
  // The plugin reports e.g. "2026-11-03 10:00:00.0 +00:00:00": the day is all we show.
  const m = /^\d{4}-\d\d-\d\d/.exec(upd.date)
  const d = m ? new Date(`${m[0]}T12:00:00`) : null
  return d && !isNaN(+d) ? d.toLocaleDateString(undefined, { day: 'numeric', month: 'long' }) : ''
})
</script>

<template>
  <Transition name="upd">
    <aside
      v-if="upd.open" role="status" aria-live="polite"
      class="upd fixed bottom-4 left-4 z-50 w-[min(360px,calc(100vw-2rem))] overflow-hidden rounded-2xl bg-panel shadow-2xl ring-1 ring-fg/8"
    >
      <!-- accent glow -->
      <div class="pointer-events-none absolute -top-16 -right-10 size-44 rounded-full bg-accent/25 blur-3xl" />
      <div class="relative p-4">
        <div class="flex items-start gap-3">
          <img :src="logo" alt="" class="size-11 shrink-0 drop-shadow-md" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="rounded-full bg-accent/15 px-2 py-0.5 text-[10.5px] font-semibold tracking-wide text-accent uppercase">{{ t('update.badge') }}</span>
              <span v-if="date" class="text-[11px] text-faint">{{ date }}</span>
            </div>
            <h3 class="mt-1 text-[14.5px] font-semibold">{{ t('update.title', { version: upd.version }) }}</h3>
            <p class="text-[11.5px] text-faint">{{ t('update.current', { version: upd.current }) }}</p>
          </div>
          <button v-if="!busy" class="-mt-1 -mr-1 grid size-6 place-items-center rounded-md text-faint hover:bg-raised hover:text-fg" :title="t('update.later')" @click="later">
            <Icon name="x" class="size-3.5" />
          </button>
        </div>

        <ul v-if="upd.changes.length" class="mt-3 space-y-1.5 rounded-xl bg-raised/60 p-3 text-[12px] text-dim">
          <li v-for="(c, i) in shown" :key="i" class="flex gap-2">
            <Icon name="check" class="mt-0.5 size-3.5 shrink-0 text-accent" /><span class="min-w-0">{{ c }}</span>
          </li>
          <li v-if="upd.changes.length > 3">
            <button class="text-[11.5px] text-accent hover:underline" @click="expanded = !expanded">
              {{ expanded ? t('update.less') : t('update.more', { count: upd.changes.length - 3 }) }}
            </button>
          </li>
        </ul>

        <div v-if="busy" class="mt-3">
          <div class="h-1.5 overflow-hidden rounded-full bg-line">
            <div class="h-full rounded-full bg-accent transition-[width]" :class="!upd.total && 'w-1/3 animate-pulse'" :style="upd.total ? { width: `${pct}%` } : {}" />
          </div>
          <p class="mt-1.5 text-[11px] text-faint">
            <template v-if="upd.phase === 'installing'">{{ t('update.installing') }}</template>
            <template v-else-if="upd.total">{{ t('update.downloading', { done: fmtSize(upd.done), total: fmtSize(upd.total) }) }}</template>
            <template v-else>{{ t('update.starting') }}</template>
          </p>
        </div>
        <p v-if="upd.phase === 'error'" class="mt-3 flex items-start gap-1.5 text-[12px] text-danger">
          <Icon name="alert-circle" class="mt-0.5 size-4 shrink-0" /><span class="min-w-0 break-words">{{ t('update.failed') }} {{ upd.error }}</span>
        </p>

        <template v-if="!busy">
          <button
            class="mt-4 flex h-9 w-full items-center justify-center gap-1.5 rounded-lg bg-accent px-3.5 text-[12.5px] font-medium whitespace-nowrap text-on-accent shadow-sm hover:brightness-110"
            @click="install"
          >
            <Icon :name="upd.mode === 'auto' ? 'refresh' : 'download'" class="size-3.5" />
            {{ upd.phase === 'error' ? t('update.retry') : upd.mode === 'auto' ? t('update.install') : t('update.download') }}
          </button>
          <div class="mt-1.5 flex items-center justify-between">
            <button class="h-7 rounded-md px-2 text-[12px] whitespace-nowrap text-dim hover:bg-raised hover:text-fg" @click="openNotes">{{ t('update.notes') }}</button>
            <button class="h-7 rounded-md px-2 text-[11.5px] whitespace-nowrap text-faint hover:text-dim" @click="skip">{{ t('update.skip') }}</button>
          </div>
        </template>
        <p v-if="upd.mode === 'manual' && !busy" class="mt-2 text-[11px] text-faint">{{ t('update.manualHint') }}</p>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.upd-enter-active,
.upd-leave-active {
  transition: opacity 0.25s ease, transform 0.35s cubic-bezier(0.2, 0.9, 0.3, 1.2);
}
.upd-enter-from,
.upd-leave-to {
  opacity: 0;
  transform: translateY(16px) scale(0.97);
}
:global([data-motion='off']) .upd-enter-active,
:global([data-motion='off']) .upd-leave-active {
  transition: none;
}
</style>
