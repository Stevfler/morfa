<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as pickFolder } from '@tauri-apps/plugin-dialog'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import Segmented from '../settings/Segmented.vue'
import { state, pane, go, toast, cloudPath } from '../../lib/store'
import type { CloudAccount } from '../../lib/store'
import { refreshCloud } from '../../lib/cloud'
import { join } from '../../lib/util'
import { t } from '../../lib/i18n'

const kind = ref<'nextcloud' | 'gdrive'>('nextcloud')
const providers = ref<string[]>(['nextcloud'])
const gdrive = computed(() => kind.value === 'gdrive')
const PROVIDERS = [{ value: 'nextcloud', label: 'Nextcloud' }, { value: 'gdrive', label: 'Google Drive' }]
const server = ref('')
const folder = ref('')
const error = ref('')
const waiting = ref(false)
const open = computed({
  get: () => state.cloudConnect,
  set: (v) => {
    if (!v && waiting.value) invoke('cloud_connect_cancel')
    state.cloudConnect = v
  },
})
watch(open, (v) => {
  if (!v) return
  error.value = ''
  invoke<string[]>('cloud_providers').then((p) => (providers.value = p), () => {})
  folder.value = defaultFolder()
})
// Keep a folder the user picked; swap only our own suggestion when the service changes.
const suggested = new Set<string>()
function defaultFolder() {
  const base = gdrive.value ? 'Google Drive' : 'Nextcloud'
  const taken = new Set(state.cloud.map((a) => a.localRoot))
  let path = join(state.home, base)
  for (let i = 2; taken.has(path); i++) path = join(state.home, `${base} ${i}`)
  suggested.add(path)
  return path
}
watch(kind, () => {
  error.value = ''
  if (!folder.value || suggested.has(folder.value)) folder.value = defaultFolder()
})
const ready = computed(() => !!folder.value.trim() && (gdrive.value ? providers.value.includes('gdrive') : !!server.value.trim()))

async function choose() {
  const dir = await pickFolder({ directory: true, defaultPath: state.home }).catch(() => null)
  if (typeof dir === 'string') folder.value = dir
}

async function submit() {
  if (!ready.value || waiting.value) return
  waiting.value = true
  error.value = ''
  try {
    const a = await invoke<CloudAccount>('cloud_connect', { kind: kind.value, server: server.value, localRoot: folder.value.trim() })
    await refreshCloud()
    state.cloudConnect = false
    server.value = ''
    folder.value = ''
    toast({ title: t('cloud.connected', { name: a.name }), tone: 'ok' }, 3000)
    go(pane(), cloudPath(a.id))
  } catch (e) {
    if (String(e) !== 'Cancelled') error.value = String(e)
  } finally {
    waiting.value = false
  }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 w-[min(460px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-xl bg-panel p-5 shadow-2xl ring-1 ring-fg/5 outline-none">
        <DialogTitle class="flex items-center gap-2 text-[15px] font-medium">
          <Icon name="cloud" class="size-4 text-dim" />{{ t('cloud.connectTitle') }}
        </DialogTitle>
        <DialogDescription class="mt-1 text-[12px] text-faint">{{ gdrive ? t('cloud.gdriveDesc') : t('cloud.connectDesc') }}</DialogDescription>
        <form class="mt-4 flex flex-col gap-3" @submit.prevent="submit">
          <Segmented v-model="kind" :options="PROVIDERS" :label="t('cloud.provider')" />
          <p v-if="gdrive && !providers.includes('gdrive')" class="flex items-center gap-1.5 text-[12px] text-warn"><Icon name="warning" class="size-4 shrink-0" />{{ t('cloud.gdriveMissing') }}</p>
          <label v-if="!gdrive" class="flex flex-col gap-1 text-[12px] text-dim">
            {{ t('cloud.server') }}
            <input
              v-model="server" autofocus autocomplete="url" spellcheck="false" :placeholder="t('cloud.serverPh')" :disabled="waiting"
              class="h-8 rounded-md bg-raised px-2.5 text-[12.5px] text-fg outline-none focus:ring-1 focus:ring-accent/50"
            />
          </label>
          <label class="flex flex-col gap-1 text-[12px] text-dim">
            {{ t('cloud.folder') }}
            <span class="flex gap-2">
              <input
                v-model="folder" spellcheck="false" :disabled="waiting"
                class="h-8 min-w-0 flex-1 rounded-md bg-raised px-2.5 font-mono text-[12px] text-fg outline-none focus:ring-1 focus:ring-accent/50"
              />
              <button type="button" class="h-8 shrink-0 rounded-md bg-raised px-3 text-[12.5px] hover:bg-line" :disabled="waiting" @click="choose">{{ t('cloud.choose') }}</button>
            </span>
            <span class="text-[11.5px] text-faint">{{ t('cloud.folderHint') }}</span>
          </label>
          <p v-if="waiting" class="flex items-center gap-1.5 text-[12px] text-dim"><Icon name="loader" class="size-4 animate-spin" />{{ t('cloud.waiting') }}</p>
          <p v-if="error" class="flex items-center gap-1.5 text-[12px] text-danger"><Icon name="alert-circle" class="size-4 shrink-0" />{{ error }}</p>
          <div class="mt-2 flex justify-end gap-2">
            <DialogClose type="button" class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">{{ t('common.cancel') }}</DialogClose>
            <button
              type="submit" :disabled="!ready || waiting"
              class="flex h-8 items-center gap-1.5 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent hover:brightness-110 disabled:opacity-40"
            ><Icon name="external-link" class="size-3.5" />{{ gdrive ? t('cloud.gdriveSignIn') : t('cloud.signIn') }}</button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
