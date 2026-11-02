<script setup lang="ts">
import { settings } from '../../lib/settings'
import { state } from '../../lib/store'
import { t } from '../../lib/i18n'
import Row from './Row.vue'
import Toggle from './Toggle.vue'
</script>

<template>
  <section class="flex flex-col gap-1">
    <Row :title="t('settings.showHidden')" :desc="t('settings.showHiddenDesc')">
      <Toggle v-model="state.showHidden" :label="t('settings.showHidden')" />
    </Row>
    <Row :title="t('settings.gitignore')" :desc="t('settings.gitignoreDesc')">
      <Toggle v-model="settings.respectGitignore" :label="t('settings.gitignore')" />
    </Row>
    <Row :title="t('settings.cleanupWarn')" :desc="t('settings.cleanupWarnDesc')">
      <Toggle :model-value="!settings.cleanupWarned" :label="t('settings.cleanupWarn')" @update:model-value="settings.cleanupWarned = !$event" />
    </Row>
  </section>
</template>
