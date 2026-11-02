<script setup lang="ts">
import { ToastProvider, ToastRoot, ToastTitle, ToastDescription, ToastAction, ToastClose, ToastViewport } from 'reka-ui'
import Icon from './Icon.vue'
import { state, dismiss, cancelOp } from '../lib/store'
import { basename, fmtSize } from '../lib/util'
import { t } from '../lib/i18n'

const ICON = { ok: 'check-circle', error: 'alert-circle', info: 'info' } as const
</script>

<template>
  <ToastProvider :duration="1e9" swipe-direction="right">
    <!-- running copy/move operations -->
    <div class="fixed right-4 bottom-4 z-50 flex w-96 flex-col gap-2">
      <div v-for="op in state.ops" :key="op.id" class="rounded-xl bg-raised p-3 ring-1 ring-fg/5 shadow-2xl">
        <div class="flex items-center gap-2 text-[12.5px]">
          <span class="flex-1 truncate">{{ op.label }}</span>
          <button class="text-[12px] text-dim hover:text-danger" @click="cancelOp(op.id)">{{ t('common.cancel') }}</button>
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-line">
          <div class="h-full rounded-full bg-accent transition-[width]" :style="{ width: op.total ? `${(op.done / op.total) * 100}%` : '8%' }" />
        </div>
        <p class="mt-1.5 truncate text-[11px] text-faint">
          <template v-if="op.total && op.unit === 'files'">{{ t('ops.files', { done: op.done, count: op.total }) }} · </template>
          <template v-else-if="op.total">{{ t('ops.bytes', { done: fmtSize(op.done), total: fmtSize(op.total) }) }} · </template>{{ op.current ? basename(op.current) : t('ops.preparing') }}
        </p>
      </div>
    </div>

    <ToastRoot
      v-for="m in state.toasts" :key="m.id" :open="true"
      class="flex items-start gap-3 rounded-xl bg-raised p-3 ring-1 ring-fg/5 shadow-2xl data-[state=closed]:animate-out data-[swipe=end]:translate-x-full"
      @update:open="(o: boolean) => !o && dismiss(m.id)"
    >
      <Icon :name="ICON[m.tone ?? 'info']" class="mt-0.5 size-4 shrink-0" :class="{ 'text-ok': m.tone === 'ok', 'text-danger': m.tone === 'error', 'text-accent': !m.tone || m.tone === 'info' }" />
      <div class="min-w-0 flex-1">
        <ToastTitle class="text-[12.5px] font-medium break-words text-fg">{{ m.title }}</ToastTitle>
        <ToastDescription v-if="m.desc" class="mt-0.5 text-[11.5px] break-words text-dim">{{ m.desc }}</ToastDescription>
      </div>
      <ToastAction v-if="m.action" :alt-text="m.action.label" as-child>
        <button class="h-6 shrink-0 rounded-full bg-accent-soft px-2.5 text-[12px] text-accent hover:bg-accent-soft" @click="m.action.run(); dismiss(m.id)">
          {{ m.action.label }}
        </button>
      </ToastAction>
      <ToastClose class="text-faint hover:text-fg"><Icon name="x" class="size-3.5" /></ToastClose>
    </ToastRoot>
    <ToastViewport class="fixed bottom-4 left-1/2 z-50 flex w-[420px] max-w-[92vw] -translate-x-1/2 flex-col gap-2 outline-none" />
  </ToastProvider>
</template>
