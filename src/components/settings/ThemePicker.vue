<script setup lang="ts">
import { RadioGroupRoot, RadioGroupItem } from 'reka-ui'
import { settings, THEMES, ACCENTS } from '../../lib/settings'
import { t } from '../../lib/i18n'
</script>

<template>
  <RadioGroupRoot v-model="settings.theme" :aria-label="t('settings.theme')" class="grid grid-cols-3 gap-2">
    <RadioGroupItem
      v-for="th in THEMES" :key="th.id" :value="th.id"
      class="group rounded-lg bg-raised/50 p-1.5 text-left outline-none transition hover:bg-raised focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:ring-2 data-[state=checked]:ring-accent"
    >
      <!-- mini window preview drawn in the theme's own colors -->
      <div class="flex h-14 overflow-hidden rounded-md border" :style="{ background: th.c[0], borderColor: th.c[3] }">
        <div class="w-1/4 space-y-1 p-1.5" :style="{ background: th.c[1] }">
          <div class="h-1 rounded-full" :style="{ background: th.c[5] }" />
          <div class="h-1 w-2/3 rounded-full" :style="{ background: th.c[6] }" />
        </div>
        <div class="flex-1 space-y-1 p-1.5">
          <div class="h-2 rounded-sm" :style="{ background: 'var(--color-accent-soft)' }" />
          <div class="h-1 w-3/4 rounded-full" :style="{ background: th.c[4] }" />
          <div class="h-1 w-1/2 rounded-full" :style="{ background: th.c[5] }" />
        </div>
      </div>
      <div class="mt-1.5 px-0.5 text-[12px] text-dim group-data-[state=checked]:text-fg">{{ th.name }}</div>
    </RadioGroupItem>
  </RadioGroupRoot>

  <RadioGroupRoot v-model="settings.accent" orientation="horizontal" :aria-label="t('settings.accent')" class="mt-4 flex flex-wrap gap-2">
    <RadioGroupItem
      v-for="a in ACCENTS" :key="a.id" :value="a.id" :aria-label="a.name" :title="a.name"
      class="size-7 rounded-full outline-none ring-offset-2 ring-offset-panel transition hover:scale-110 focus-visible:ring-2 focus-visible:ring-fg/60 data-[state=checked]:ring-2 data-[state=checked]:ring-fg"
      :style="{ background: a.hex }"
    />
  </RadioGroupRoot>
</template>
