<script setup lang="ts">
import { computed } from 'vue'
import { settings, SCALES } from '../../lib/settings'
import { t } from '../../lib/i18n'
import ThemePicker from './ThemePicker.vue'
import Segmented from './Segmented.vue'
import Row from './Row.vue'
import LanguageRow from './LanguageRow.vue'

const opts = <K extends string>(prefix: string, vals: K[]) => vals.map((value) => ({ value, label: t(`${prefix}.${value}` as Parameters<typeof t>[0]) }))
const radii = computed(() => opts('settings.radius', ['sharp', 'default', 'round', 'extra']))
const motions = computed(() => opts('settings.motion', ['full', 'reduced', 'off']))
const scales = SCALES.map((s) => ({ value: String(s), label: `${Math.round(s * 100)}%` }))
const densities = computed(() => opts('settings.density', ['comfortable', 'compact']))
</script>

<template>
  <section class="space-y-5">
    <LanguageRow />
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">{{ t('settings.themeAccent') }}</h3>
      <ThemePicker />
    </div>
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">{{ t('settings.corners') }}</h3>
      <Segmented v-model="settings.radius" :options="radii" :label="t('settings.cornerRadius')" />
      <!-- live preview: follows the radius tokens -->
      <div class="mt-3 flex items-center gap-2 rounded-xl bg-raised p-3">
        <div class="h-8 flex-1 rounded-lg bg-raised" />
        <div class="grid h-8 place-items-center rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent">{{ t('settings.button') }}</div>
        <div class="size-8 rounded-md ring-1 ring-fg/10" />
      </div>
    </div>
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">{{ t('settings.animations') }}</h3>
      <Segmented v-model="settings.motion" :options="motions" :label="t('settings.animations')" />
    </div>
    <Row :title="t('settings.scale')" :desc="t('settings.scaleDesc')">
      <Segmented :model-value="String(settings.scale)" :options="scales" :label="t('settings.scale')" class="w-72" @update:model-value="settings.scale = Number($event)" />
    </Row>
    <Row :title="t('settings.density')" :desc="t('settings.densityDesc')">
      <Segmented :model-value="settings.density ?? 'comfortable'" :options="densities" :label="t('settings.density')" @update:model-value="settings.density = $event as 'compact' | 'comfortable'" />
    </Row>
  </section>
</template>
