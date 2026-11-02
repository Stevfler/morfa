<script setup lang="ts">
import { RadioGroupRoot, RadioGroupItem } from 'reka-ui'
import { settings, THEMES, ACCENTS } from '../../lib/settings'
</script>

<template>
  <RadioGroupRoot v-model="settings.theme" aria-label="Theme" class="grid grid-cols-3 gap-2">
    <RadioGroupItem
      v-for="t in THEMES" :key="t.id" :value="t.id"
      class="group rounded-lg border border-line p-1.5 text-left outline-none transition hover:border-faint focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:border-accent"
    >
      <!-- mini window preview drawn in the theme's own colors -->
      <div class="flex h-14 overflow-hidden rounded-md border" :style="{ background: t.c[0], borderColor: t.c[3] }">
        <div class="w-1/4 space-y-1 p-1.5" :style="{ background: t.c[1] }">
          <div class="h-1 rounded-full" :style="{ background: t.c[5] }" />
          <div class="h-1 w-2/3 rounded-full" :style="{ background: t.c[6] }" />
        </div>
        <div class="flex-1 space-y-1 p-1.5">
          <div class="h-2 rounded-sm" :style="{ background: 'var(--color-accent-soft)' }" />
          <div class="h-1 w-3/4 rounded-full" :style="{ background: t.c[4] }" />
          <div class="h-1 w-1/2 rounded-full" :style="{ background: t.c[5] }" />
        </div>
      </div>
      <div class="mt-1.5 px-0.5 text-[12px] text-dim group-data-[state=checked]:text-fg">{{ t.name }}</div>
    </RadioGroupItem>
  </RadioGroupRoot>

  <RadioGroupRoot v-model="settings.accent" orientation="horizontal" aria-label="Accent color" class="mt-4 flex flex-wrap gap-2">
    <RadioGroupItem
      v-for="a in ACCENTS" :key="a.id" :value="a.id" :aria-label="a.name" :title="a.name"
      class="size-7 rounded-full outline-none ring-offset-2 ring-offset-panel transition hover:scale-110 focus-visible:ring-2 focus-visible:ring-fg/60 data-[state=checked]:ring-2 data-[state=checked]:ring-fg"
      :style="{ background: a.hex }"
    />
  </RadioGroupRoot>
</template>
