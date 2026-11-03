<script setup lang="ts">
import { ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { t } from '../../lib/i18n'
import Row from './Row.vue'
import Toggle from './Toggle.vue'
import Icon from '../Icon.vue'
import { settings } from '../../lib/settings'
import { upd, checkUpdates } from '../../lib/update'
import { openUrl } from '@tauri-apps/plugin-opener'
import { toast } from '../../lib/store'

const TIPS = 'https://pay.cloudtips.ru/p/9208e282'
const USDT = 'TMArKAucw4yNhx4wt8oHJ5Ke5VKL9SE9tH'
async function copyUsdt() {
  await navigator.clipboard.writeText(USDT)
  toast({ title: t('support.copied'), tone: 'ok' }, 2500)
}
import logo from '../../../src-tauri/icons/128x128@2x.png'

// The app version comes from package.json (set from the release tag in CI).
const version = ref(__APP_VERSION__)
if ('__TAURI_INTERNALS__' in window) {
  getVersion().then((v) => (version.value = v)).catch(() => {})
}
</script>

<template>
  <section class="space-y-5">
    <div class="flex items-center gap-4">
      <img :src="logo" alt="" class="size-16" />
      <div>
        <div class="text-[18px] font-semibold">Morfa</div>
        <div class="mt-0.5 text-[12.5px] text-dim">{{ t('settings.about.tagline') }}</div>
      </div>
    </div>
    <div class="divide-y divide-line">
      <Row :title="t('settings.about.version')">
        <span class="flex items-center gap-3">
          <span class="text-[12.5px] tabular-nums text-dim select-text">{{ version }}</span>
          <button
            class="flex h-7 items-center gap-1.5 rounded-md bg-raised px-2.5 text-[12px] hover:bg-line disabled:opacity-50"
            :disabled="upd.phase === 'checking'" @click="checkUpdates(true)"
          ><Icon name="refresh" class="size-3.5" :class="upd.phase === 'checking' && 'animate-spin'" />{{ t('update.check') }}</button>
        </span>
      </Row>
      <Row :title="t('update.auto')" :desc="t('update.autoDesc')">
        <Toggle v-model="settings.checkUpdates" :label="t('update.auto')" />
      </Row>
    </div>
    <div class="rounded-xl bg-raised/60 p-4">
      <div class="flex items-center gap-2 text-[13px] font-medium"><Icon name="heart" class="size-4 text-danger" />{{ t('support.title') }}</div>
      <p class="mt-1 text-[12px] text-dim">{{ t('support.desc') }}</p>
      <div class="mt-3 flex flex-wrap gap-2">
        <button class="flex h-8 items-center gap-1.5 rounded-lg bg-accent px-3 text-[12.5px] font-medium text-on-accent hover:brightness-110" @click="openUrl(TIPS)">
          <Icon name="external-link" class="size-3.5" />{{ t('support.card') }}
        </button>
        <button class="flex h-8 items-center gap-1.5 rounded-lg bg-panel px-3 text-[12.5px] hover:bg-line" :title="USDT" @click="copyUsdt">
          <Icon name="copy" class="size-3.5" />{{ t('support.usdt') }}
        </button>
      </div>
    </div>
  </section>
</template>
