<script setup lang="ts">
import { RadioGroupRoot, RadioGroupItem, RadioGroupIndicator } from 'reka-ui'
import { settings, type IconPack } from '../../lib/settings'
import type { IconName } from '../../lib/icons'
import { t } from '../../lib/i18n'
import Icon from '../Icon.vue'
import IconColorPicker from './IconColorPicker.vue'

const packs: { id: IconPack; name: string }[] = [
  { id: 'lucide', name: 'Lucide' },
  { id: 'phosphor', name: 'Phosphor' },
  { id: 'solar', name: 'Solar' },
  { id: 'fluent', name: 'Fluent' },
  { id: 'tabler', name: 'Tabler' },
]
const sample: IconName[] = ['folder', 'image', 'trash', 'search', 'settings', 'star']
</script>

<template>
  <RadioGroupRoot v-model="settings.iconPack" :aria-label="t('settings.iconPack')" class="space-y-1">
    <RadioGroupItem
      v-for="p in packs" :key="p.id" :value="p.id"
      class="flex h-10 w-full items-center gap-3 rounded-lg px-3 text-left text-[12.5px] outline-none transition hover:bg-raised focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:bg-raised"
    >
      <span class="grid size-4 place-items-center rounded-full border border-faint">
        <RadioGroupIndicator class="size-2 rounded-full bg-accent" />
      </span>
      <span class="flex-1">{{ p.name }} <span class="ml-1 text-[11.5px] text-faint">{{ t(`settings.pack.${p.id}`) }}</span></span>
      <span class="flex gap-2 text-dim" :data-icon-preview="p.id">
        <Icon v-for="n in sample" :key="n" :name="n" :pack="p.id" class="size-4" :tint="n === 'folder' ? 0.15 : 0" />
      </span>
    </RadioGroupItem>
  </RadioGroupRoot>
  <IconColorPicker />
</template>
