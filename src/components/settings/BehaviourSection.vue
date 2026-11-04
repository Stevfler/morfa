<script setup lang="ts">
import { settings } from '../../lib/settings'
import { state } from '../../lib/store'
import { t } from '../../lib/i18n'
import Row from './Row.vue'
import Toggle from './Toggle.vue'
import { confirmClear } from '../journal/journal'
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
    <Row :title="t('settings.journal')" :desc="t('settings.journalDesc')">
      <Toggle v-model="settings.journal" :label="t('settings.journal')" />
    </Row>
    <Row :title="t('settings.journalClear')" :desc="t('settings.journalClearDesc')">
      <button class="h-7 rounded-md bg-raised px-3 text-[12px] text-danger hover:bg-danger/10" @click="confirmClear">{{ t('journal.clear') }}</button>
    </Row>
  </section>
</template>
