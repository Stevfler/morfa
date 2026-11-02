<script setup lang="ts">
import { RadioGroupRoot, RadioGroupItem, RadioGroupIndicator } from 'reka-ui'
import { settings, type IconPack } from '../../lib/settings'
import type { IconName } from '../../lib/icons'
import Icon from '../Icon.vue'
import IconColorPicker from './IconColorPicker.vue'

const packs: { id: IconPack; name: string; note: string }[] = [
  { id: 'lucide', name: 'Lucide', note: 'Clean outline' },
  { id: 'phosphor', name: 'Phosphor', note: 'Duotone, soft' },
  { id: 'solar', name: 'Solar', note: 'Bold duotone, puffy' },
  { id: 'fluent', name: 'Fluent', note: 'Rounded, Windows-like' },
  { id: 'tabler', name: 'Tabler', note: 'Friendly outline' },
]
const sample: IconName[] = ['folder', 'image', 'trash', 'search', 'settings', 'star']
</script>

<template>
  <RadioGroupRoot v-model="settings.iconPack" aria-label="Icon pack" class="space-y-1">
    <RadioGroupItem
      v-for="p in packs" :key="p.id" :value="p.id"
      class="flex h-10 w-full items-center gap-3 rounded-lg border border-transparent px-3 text-left text-[12.5px] outline-none transition hover:bg-raised focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:border-line data-[state=checked]:bg-raised"
    >
      <span class="grid size-4 place-items-center rounded-full border border-faint">
        <RadioGroupIndicator class="size-2 rounded-full bg-accent" />
      </span>
      <span class="flex-1">{{ p.name }} <span class="ml-1 text-[11.5px] text-faint">{{ p.note }}</span></span>
      <span class="flex gap-2 text-dim" :data-icon-preview="p.id">
        <Icon v-for="n in sample" :key="n" :name="n" :pack="p.id" class="size-4" :tint="n === 'folder' ? 0.15 : 0" />
      </span>
    </RadioGroupItem>
  </RadioGroupRoot>
  <IconColorPicker />
</template>
