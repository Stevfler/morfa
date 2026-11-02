<script setup lang="ts">
import { settings } from '../../lib/settings'
import ThemePicker from './ThemePicker.vue'
import Segmented from './Segmented.vue'
import Row from './Row.vue'

const radii = [
  { value: 'sharp', label: 'Sharp' }, { value: 'default', label: 'Default' },
  { value: 'round', label: 'Round' }, { value: 'extra', label: 'Extra' },
]
const motions = [{ value: 'full', label: 'Full' }, { value: 'reduced', label: 'Reduced' }, { value: 'off', label: 'Off' }]
const densities = [{ value: 'comfortable', label: 'Comfortable' }, { value: 'compact', label: 'Compact' }]
</script>

<template>
  <section class="space-y-5">
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">Theme & accent</h3>
      <ThemePicker />
    </div>
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">Corners</h3>
      <Segmented v-model="settings.radius" :options="radii" label="Corner radius" />
      <!-- live preview: follows the radius tokens -->
      <div class="mt-3 flex items-center gap-2 rounded-xl bg-raised p-3">
        <div class="h-8 flex-1 rounded-lg bg-raised" />
        <div class="grid h-8 place-items-center rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent">Button</div>
        <div class="size-8 rounded-md ring-1 ring-fg/10" />
      </div>
    </div>
    <div>
      <h3 class="mb-2 text-[11px] font-medium uppercase tracking-wide text-faint">Animations</h3>
      <Segmented v-model="settings.motion" :options="motions" label="Animations" />
    </div>
    <Row title="Density" desc="Row height in file lists">
      <Segmented :model-value="settings.density ?? 'comfortable'" :options="densities" label="Density" @update:model-value="settings.density = $event as 'compact' | 'comfortable'" />
    </Row>
  </section>
</template>
