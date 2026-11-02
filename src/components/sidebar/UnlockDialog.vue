<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import { fmtSize } from '../../lib/util'
import { drives, mountDrive } from './drives'
import { t } from '../../lib/i18n'

// The passphrase lives only in this input; it is cleared on close and never stored.
const pass = ref('')
const error = ref('')
const working = ref(false)
const open = computed({ get: () => !!drives.unlock, set: (v) => !v && (drives.unlock = null) })
watch(open, () => ((pass.value = ''), (error.value = '')))

async function submit() {
  const d = drives.unlock
  if (!d || !pass.value || working.value) return
  working.value = true
  error.value = ''
  try {
    await mountDrive(d, pass.value)
    if (drives.unlock === d) drives.unlock = null
  } catch (e) {
    // Ignore results for a dialog the user already cancelled.
    if (drives.unlock === d) ((error.value = String(e)), (pass.value = ''))
  } finally {
    working.value = false
  }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 w-[min(400px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-xl bg-panel p-5 shadow-2xl ring-1 ring-fg/5 outline-none">
        <DialogTitle class="flex items-center gap-2 text-[15px] font-medium">
          <Icon name="lock" class="size-4 text-dim" />{{ t('unlock.title', { name: drives.unlock?.name || t('unlock.encrypted') }) }}
        </DialogTitle>
        <DialogDescription class="mt-1 text-[12px] text-faint">
          {{ drives.unlock?.device }} · {{ fmtSize(drives.unlock?.size ?? 0) }}. {{ t('unlock.desc') }}
        </DialogDescription>
        <form class="mt-4" @submit.prevent="submit">
          <input
            v-model="pass" type="password" autofocus autocomplete="off" spellcheck="false" :placeholder="t('unlock.pass')"
            :disabled="working"
            class="h-8 w-full rounded-md bg-raised px-2.5 text-[12.5px] outline-none focus:ring-1 focus:ring-accent/50"
          />
          <p v-if="error" class="mt-2 flex items-center gap-1.5 text-[12px] text-danger"><Icon name="alert-circle" class="size-4" />{{ error }}</p>
          <div class="mt-5 flex justify-end gap-2">
            <DialogClose type="button" class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">{{ t('common.cancel') }}</DialogClose>
            <button
              type="submit" :disabled="!pass || working"
              class="flex h-8 items-center gap-1.5 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent hover:brightness-110 disabled:opacity-40"
            ><Icon v-if="working" name="loader" class="size-3.5 animate-spin" />{{ t('unlock.unlock') }}</button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
