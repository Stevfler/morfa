<script setup lang="ts">
import { ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { t } from '../../lib/i18n'
import Row from './Row.vue'
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
      <Row :title="t('settings.about.version')"><span class="text-[12.5px] tabular-nums text-dim select-text">{{ version }}</span></Row>
    </div>
  </section>
</template>
