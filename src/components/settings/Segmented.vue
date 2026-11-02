<script setup lang="ts">
import { RadioGroupRoot, RadioGroupItem } from 'reka-ui'
defineProps<{ options: { value: string; label: string }[]; label: string }>()
const model = defineModel<string>({ required: true })
</script>

<template>
  <RadioGroupRoot v-model="model" orientation="horizontal" :aria-label="label" class="flex gap-1 rounded-lg border border-line bg-bg p-1">
    <RadioGroupItem
      v-for="o in options" :key="o.value" :value="o.value"
      class="h-7 flex-1 rounded-md px-2 text-[12px] text-dim outline-none transition hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:bg-raised data-[state=checked]:text-fg data-[state=checked]:shadow-sm"
    >{{ o.label }}</RadioGroupItem>
  </RadioGroupRoot>
</template>
