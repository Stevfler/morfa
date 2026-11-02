<script setup lang="ts">
import { computed } from 'vue'
import {
  AlertDialogRoot, AlertDialogPortal, AlertDialogOverlay, AlertDialogContent, AlertDialogTitle,
  AlertDialogDescription, AlertDialogCancel, AlertDialogAction,
} from 'reka-ui'
import { state } from '../lib/store'

const open = computed({ get: () => !!state.confirm, set: (v) => !v && (state.confirm = null) })
function ok() {
  const c = state.confirm
  state.confirm = null
  c?.run()
}
</script>

<template>
  <AlertDialogRoot v-model:open="open">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <AlertDialogContent class="fixed top-1/2 left-1/2 z-50 w-[min(440px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-xl bg-panel ring-1 ring-fg/5 p-5 shadow-2xl outline-none">
        <AlertDialogTitle class="text-[15px] font-medium">{{ state.confirm?.title }}</AlertDialogTitle>
        <AlertDialogDescription class="mt-2 text-[12.5px] leading-relaxed text-dim">{{ state.confirm?.desc }}</AlertDialogDescription>
        <div class="mt-5 flex justify-end gap-2">
          <AlertDialogCancel class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">Cancel</AlertDialogCancel>
          <AlertDialogAction
            class="h-8 rounded-md px-4 text-[12.5px] font-medium hover:brightness-110"
            :class="state.confirm?.danger ? 'bg-danger text-white' : 'bg-accent text-on-accent'" @click="ok"
          >{{ state.confirm?.ok }}</AlertDialogAction>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
