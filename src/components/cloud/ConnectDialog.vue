<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as pickPath } from '@tauri-apps/plugin-dialog'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import { state, pane, go, toast, cloudPath, parseHostKeyChange } from '../../lib/store'
import type { CloudAccount } from '../../lib/store'
import { refreshCloud } from '../../lib/cloud'
import { join } from '../../lib/util'
import { t } from '../../lib/i18n'

type Kind = 'nextcloud' | 'webdav' | 'sftp'
const KINDS: Kind[] = ['nextcloud', 'webdav', 'sftp']
const kindLabel = (k: Kind) => t(k === 'nextcloud' ? 'cloud.kindNextcloud' : k === 'webdav' ? 'cloud.kindWebdav' : 'cloud.kindSftp')

const kind = ref<Kind>('nextcloud')
const server = ref('')
const login = ref('')
const password = ref('')
const insecureOk = ref(false)
const port = ref(22)
const useKey = ref(false)
const keyPath = ref('')
const passphrase = ref('')
const remotePath = ref('')
/** The fingerprint the server showed for this host and port, and whether the user confirmed it. */
const fingerprint = ref('')
const fingerprintOk = ref(false)
const folder = ref('')
const error = ref('')
const waiting = ref(false)

const open = computed({
  get: () => state.cloudConnect,
  set: (v) => {
    if (!v && waiting.value) invoke('cloud_connect_cancel')
    if (!v) forgetSecrets()
    state.cloudConnect = v
  },
})
function forgetSecrets() {
  password.value = ''
  passphrase.value = ''
}
watch(open, (v) => {
  if (!v) return
  error.value = ''
  if (!folder.value) folder.value = defaultFolder()
})
// A confirmation holds only for the exact server that was checked.
watch([server, port, kind], () => {
  fingerprint.value = ''
  fingerprintOk.value = false
})
watch(server, () => (insecureOk.value = false))
const usedDefault = ref(true)
watch(kind, () => {
  error.value = ''
  password.value = ''
  if (usedDefault.value) folder.value = defaultFolder()
})
watch(folder, (v) => (usedDefault.value = v === defaultFolder()))

function defaultFolder() {
  const base = kind.value === 'nextcloud' ? 'Nextcloud' : kind.value === 'webdav' ? 'WebDAV' : 'SFTP'
  const taken = new Set(state.cloud.map((a) => a.localRoot))
  let path = join(state.home, base)
  for (let i = 2; taken.has(path); i++) path = join(state.home, `${base} ${i}`)
  return path
}

const isHttp = computed(() => kind.value !== 'sftp' && /^\s*http:\/\//i.test(server.value))
const credsReady = computed(() => {
  if (kind.value === 'nextcloud') return true
  if (!login.value.trim()) return false
  if (kind.value === 'sftp' && useKey.value) return !!keyPath.value.trim()
  return !!password.value
})
const ready = computed(() => !!folder.value.trim() && !!server.value.trim() && credsReady.value && (!isHttp.value || insecureOk.value))
/** SFTP goes in two steps: check the server, confirm its fingerprint, then connect. */
const needsCheck = computed(() => kind.value === 'sftp' && !fingerprint.value)
const canSubmit = computed(() => ready.value && !waiting.value && (kind.value !== 'sftp' || !fingerprint.value || fingerprintOk.value))
const waitingText = computed(() => t(kind.value === 'nextcloud' ? 'cloud.waiting' : needsCheck.value ? 'cloud.checking' : 'cloud.connecting'))

async function choose() {
  const dir = await pickPath({ directory: true, defaultPath: state.home }).catch(() => null)
  if (typeof dir === 'string') folder.value = dir
}
async function chooseKey() {
  const f = await pickPath({ directory: false, defaultPath: join(state.home, '.ssh') }).catch(() => null)
  if (typeof f === 'string') keyPath.value = f
}

function connect(): Promise<CloudAccount> {
  const localRoot = folder.value.trim()
  if (kind.value === 'nextcloud') return invoke<CloudAccount>('cloud_connect', { server: server.value, localRoot })
  if (kind.value === 'webdav') return invoke<CloudAccount>('cloud_connect_webdav', { url: server.value, login: login.value, password: password.value, localRoot, allowInsecure: insecureOk.value })
  return invoke<CloudAccount>('cloud_connect_sftp', {
    host: server.value,
    port: port.value,
    login: login.value,
    password: useKey.value ? null : password.value,
    keyPath: useKey.value ? keyPath.value : null,
    passphrase: useKey.value ? passphrase.value : null,
    remotePath: remotePath.value,
    fingerprint: fingerprint.value,
    localRoot,
  })
}

async function submit() {
  if (!canSubmit.value) return
  waiting.value = true
  error.value = ''
  try {
    if (needsCheck.value) {
      fingerprint.value = await invoke<string>('cloud_sftp_probe', { host: server.value, port: port.value })
      return
    }
    const a = await connect()
    await refreshCloud()
    state.cloudConnect = false
    forgetSecrets()
    server.value = ''
    login.value = ''
    keyPath.value = ''
    remotePath.value = ''
    folder.value = ''
    toast({ title: t('cloud.connected', { name: a.name }), tone: 'ok' }, 3000)
    go(pane(), cloudPath(a.id))
  } catch (e) {
    const changed = parseHostKeyChange(e)
    if (changed) {
      // The server changed between the check and the sign-in: confirm again from scratch.
      fingerprint.value = ''
      fingerprintOk.value = false
      error.value = t('cloud.hostKeyChanged', changed)
    } else if (!/^cancelled$/i.test(String(e))) error.value = String(e)
  } finally {
    waiting.value = false
  }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 max-h-[90vh] w-[min(480px,92vw)] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-xl bg-panel p-5 shadow-2xl ring-1 ring-fg/5 outline-none">
        <DialogTitle class="flex items-center gap-2 text-[15px] font-medium">
          <Icon name="cloud" class="size-4 text-dim" />{{ t('cloud.connectTitle') }}
        </DialogTitle>
        <DialogDescription class="mt-1 text-[12px] text-faint">
          {{ t(kind === 'nextcloud' ? 'cloud.connectDesc' : kind === 'webdav' ? 'cloud.webdavDesc' : 'cloud.sftpDesc') }}
        </DialogDescription>
        <div class="mt-3 flex gap-1 rounded-lg bg-raised p-0.5" role="radiogroup">
          <button
            v-for="k in KINDS" :key="k" type="button" role="radio" :aria-checked="kind === k" :disabled="waiting"
            class="h-7 flex-1 rounded-md text-[12.5px]" :class="kind === k ? 'bg-panel text-fg shadow-sm' : 'text-dim hover:text-fg'" @click="kind = k"
          >{{ kindLabel(k) }}</button>
        </div>
        <form class="mt-4 flex flex-col gap-3" autocomplete="off" @submit.prevent="submit">
          <div class="flex gap-2">
            <label class="flex min-w-0 flex-1 flex-col gap-1 text-[12px] text-dim">
              {{ t(kind === 'webdav' ? 'cloud.url' : kind === 'sftp' ? 'cloud.host' : 'cloud.server') }}
              <input
                v-model="server" autofocus spellcheck="false" autocapitalize="off" :disabled="waiting"
                :placeholder="t(kind === 'webdav' ? 'cloud.urlPh' : kind === 'sftp' ? 'cloud.hostPh' : 'cloud.serverPh')" class="field"
              />
            </label>
            <label v-if="kind === 'sftp'" class="flex w-20 flex-col gap-1 text-[12px] text-dim">
              {{ t('cloud.port') }}
              <input v-model.number="port" type="number" min="1" max="65535" :disabled="waiting" class="field" />
            </label>
          </div>
          <template v-if="isHttp">
            <p class="flex items-start gap-1.5 text-[12px] text-danger"><Icon name="alert-circle" class="mt-px size-4 shrink-0" />{{ t('cloud.httpWarning') }}</p>
            <label class="flex items-center gap-2 text-[12px] text-dim"><input v-model="insecureOk" type="checkbox" :disabled="waiting" />{{ t('cloud.httpConfirm') }}</label>
          </template>
          <template v-if="kind !== 'nextcloud'">
            <label class="flex flex-col gap-1 text-[12px] text-dim">
              {{ t('cloud.login') }}
              <input v-model="login" spellcheck="false" autocapitalize="off" :disabled="waiting" class="field" />
            </label>
            <div v-if="kind === 'sftp'" class="flex gap-4 text-[12px] text-dim">
              <label class="flex items-center gap-1.5"><input v-model="useKey" type="radio" :value="false" :disabled="waiting" />{{ t('cloud.authPassword') }}</label>
              <label class="flex items-center gap-1.5"><input v-model="useKey" type="radio" :value="true" :disabled="waiting" />{{ t('cloud.authKey') }}</label>
            </div>
            <template v-if="kind === 'sftp' && useKey">
              <label class="flex flex-col gap-1 text-[12px] text-dim">
                {{ t('cloud.keyFile') }}
                <span class="flex gap-2">
                  <input v-model="keyPath" spellcheck="false" :disabled="waiting" class="field min-w-0 flex-1 font-mono" />
                  <button type="button" class="h-8 shrink-0 rounded-md bg-raised px-3 text-[12.5px] hover:bg-line" :disabled="waiting" @click="chooseKey">{{ t('cloud.choose') }}</button>
                </span>
              </label>
              <label class="flex flex-col gap-1 text-[12px] text-dim">
                {{ t('cloud.passphrase') }}
                <input v-model="passphrase" type="password" autocomplete="off" :disabled="waiting" class="field" />
              </label>
            </template>
            <label v-else class="flex flex-col gap-1 text-[12px] text-dim">
              {{ t(kind === 'webdav' ? 'cloud.appPassword' : 'cloud.password') }}
              <input v-model="password" type="password" autocomplete="off" :disabled="waiting" class="field" />
            </label>
            <label v-if="kind === 'sftp'" class="flex flex-col gap-1 text-[12px] text-dim">
              {{ t('cloud.remotePath') }}
              <input v-model="remotePath" spellcheck="false" :placeholder="t('cloud.remotePathPh')" :disabled="waiting" class="field font-mono" />
            </label>
          </template>
          <label class="flex flex-col gap-1 text-[12px] text-dim">
            {{ t('cloud.folder') }}
            <span class="flex gap-2">
              <input v-model="folder" spellcheck="false" :disabled="waiting" class="field min-w-0 flex-1 font-mono" />
              <button type="button" class="h-8 shrink-0 rounded-md bg-raised px-3 text-[12.5px] hover:bg-line" :disabled="waiting" @click="choose">{{ t('cloud.choose') }}</button>
            </span>
            <span class="text-[11.5px] text-faint">{{ t('cloud.folderHint') }}</span>
            <span v-if="kind !== 'nextcloud'" class="text-[11.5px] text-faint">{{ t('cloud.noTrashHint') }}</span>
          </label>
          <div v-if="kind === 'sftp' && fingerprint" class="flex flex-col gap-1.5 rounded-lg bg-raised p-3 text-[12px]">
            <p class="font-medium text-fg">{{ t('cloud.fingerprintTitle') }}</p>
            <p class="text-faint">{{ t('cloud.fingerprintDesc') }}</p>
            <code class="font-mono text-[11.5px] break-all text-fg select-all">{{ fingerprint }}</code>
            <label class="mt-1 flex items-center gap-2 text-dim"><input v-model="fingerprintOk" type="checkbox" :disabled="waiting" />{{ t('cloud.fingerprintConfirm') }}</label>
          </div>
          <p v-if="waiting" class="flex items-center gap-1.5 text-[12px] text-dim"><Icon name="loader" class="size-4 animate-spin" />{{ waitingText }}</p>
          <p v-if="error" class="flex items-start gap-1.5 text-[12px] text-danger"><Icon name="alert-circle" class="size-4 shrink-0" />{{ error }}</p>
          <div class="mt-2 flex justify-end gap-2">
            <DialogClose type="button" class="h-8 rounded-md px-3 text-[12.5px] text-dim hover:bg-raised">{{ t('common.cancel') }}</DialogClose>
            <button
              type="submit" :disabled="!canSubmit"
              class="flex h-8 items-center gap-1.5 rounded-md bg-accent px-4 text-[12.5px] font-medium text-on-accent hover:brightness-110 disabled:opacity-40"
            >
              <Icon v-if="kind === 'nextcloud'" name="external-link" class="size-3.5" />
              {{ t(kind === 'nextcloud' ? 'cloud.signIn' : needsCheck ? 'cloud.checkServer' : 'cloud.connect') }}
            </button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
@reference '../../style.css';
.field {
  @apply h-8 rounded-md bg-raised px-2.5 text-[12.5px] text-fg outline-none focus:ring-1 focus:ring-accent/50;
}
</style>
