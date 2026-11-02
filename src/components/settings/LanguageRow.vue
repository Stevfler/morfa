<script setup lang="ts">
import { computed } from 'vue'
import { settings } from '../../lib/settings'
import { LANGUAGES, t } from '../../lib/i18n'
import Row from './Row.vue'
import Segmented from './Segmented.vue'

const options = computed(() => [{ value: 'system', label: t('settings.languageSystem') }, ...LANGUAGES.map((l) => ({ value: l.id, label: l.label }))])
</script>

<template>
  <Row :title="t('settings.language')" :desc="t('settings.languageDesc')">
    <Segmented v-model="settings.language" :options="options" :label="t('settings.language')" />
  </Row>
</template>
