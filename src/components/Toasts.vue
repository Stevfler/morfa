<script setup lang="ts">
import { ToastProvider, ToastRoot, ToastTitle, ToastDescription, ToastAction, ToastClose, ToastViewport } from 'reka-ui'
import Icon from './Icon.vue'
import { state, dismiss, cancelOp } from '../lib/store'
import { basename, fmtSize } from '../lib/util'

const ICON = { ok: 'check-circle', error: 'alert-circle', info: 'info' } as const
</script>

<template>
  <ToastProvider :duration="1e9" swipe-direction="right">
    <!-- running copy/move operations -->
    <div class="fixed right-4 bottom-4 z-50 flex w-96 flex-col gap-2">
      <div v-for="op in state.ops" :key="op.id" class="rounded-lg border border-line bg-raised p-3 shadow-2xl">
        <div class="flex items-center gap-2 text-[12.5px]">
          <span class="flex-1 truncate">{{ op.label }}</span>
          <button class="text-[12px] text-dim hover:text-danger" @click="cancelOp(op.id)">Cancel</button>
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-line">
          <div class="h-full rounded-full bg-accent transition-[width]" :style="{ width: op.total ? `${(op.done / op.total) * 100}%` : '8%' }" />
        </div>
        <p class="mt-1.5 truncate text-[11px] text-faint">
          <template v-if="op.total">{{ fmtSize(op.done) }} of {{ fmtSize(op.total) }} · </template>{{ op.current ? basename(op.current) : 'Preparing…' }}
        </p>
      </div>
    </div>

    <ToastRoot
      v-for="t in state.toasts" :key="t.id" :open="true"
      class="flex items-start gap-3 rounded-lg border border-line bg-raised p-3 shadow-2xl data-[state=closed]:animate-out data-[swipe=end]:translate-x-full"
      @update:open="(o: boolean) => !o && dismiss(t.id)"
    >
      <Icon :name="ICON[t.tone ?? 'info']" class="mt-0.5 size-4 shrink-0" :class="{ 'text-ok': t.tone === 'ok', 'text-danger': t.tone === 'error', 'text-accent': !t.tone || t.tone === 'info' }" />
      <div class="min-w-0 flex-1">
        <ToastTitle class="text-[12.5px] font-medium break-words text-fg">{{ t.title }}</ToastTitle>
        <ToastDescription v-if="t.desc" class="mt-0.5 text-[11.5px] break-words text-dim">{{ t.desc }}</ToastDescription>
      </div>
      <ToastAction v-if="t.action" :alt-text="t.action.label" as-child>
        <button class="h-6 shrink-0 rounded-md border border-accent/50 px-2 text-[12px] text-accent hover:bg-accent-soft" @click="t.action.run(); dismiss(t.id)">
          {{ t.action.label }}
        </button>
      </ToastAction>
      <ToastClose class="text-faint hover:text-fg"><Icon name="x" class="size-3.5" /></ToastClose>
    </ToastRoot>
    <ToastViewport class="fixed bottom-4 left-1/2 z-50 flex w-[420px] max-w-[92vw] -translate-x-1/2 flex-col gap-2 outline-none" />
  </ToastProvider>
</template>
