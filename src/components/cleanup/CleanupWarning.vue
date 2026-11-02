<script setup lang="ts">
import { ref } from 'vue'
import {
  AlertDialogRoot, AlertDialogPortal, AlertDialogOverlay, AlertDialogContent, AlertDialogTitle,
  AlertDialogDescription, AlertDialogCancel, AlertDialogAction,
} from 'reka-ui'
import Icon from '../Icon.vue'
import { settings } from '../../lib/settings'
import { cleanup, start } from './cleanup'

const dontShow = ref(false)
function ok() {
  if (dontShow.value) settings.cleanupWarned = true
  start()
}
</script>

<template>
  <AlertDialogRoot v-model:open="cleanup.warn">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <AlertDialogContent class="fixed top-1/2 left-1/2 z-50 w-[min(460px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-xl bg-panel ring-1 ring-fg/5 p-5 shadow-2xl outline-none">
        <AlertDialogTitle class="flex items-center gap-2 text-[15px] font-medium">
          <Icon name="warning" class="size-4.5 text-warn" /> Clean up is for experienced users
        </AlertDialogTitle>
        <AlertDialogDescription class="mt-2 text-[12.5px] leading-relaxed text-dim">
          This tool is for experienced users. Deleting the wrong files can break your system or lose data.
          Items are moved to Trash by default.
        </AlertDialogDescription>
        <label class="mt-4 flex w-fit cursor-pointer items-center gap-2 text-[12.5px] text-dim">
          <input v-model="dontShow" type="checkbox" class="size-3.5 accent-[var(--color-accent)]" /> Don't show again
        </label>
        <div class="mt-5 flex justify-end gap-2">
          <AlertDialogCancel class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">Cancel</AlertDialogCancel>
          <AlertDialogAction class="h-8 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent hover:brightness-110" @click="ok">Continue</AlertDialogAction>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
