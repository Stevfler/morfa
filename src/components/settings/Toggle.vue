<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from 'reka-ui'
defineProps<{ label: string }>()
const model = defineModel<boolean>({ required: true })
</script>

<template>
  <SwitchRoot
    v-model="model" :aria-label="label"
    class="relative h-5 w-9 rounded-full bg-line outline-none transition focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=checked]:bg-accent"
  >
    <SwitchThumb class="block size-4 translate-x-0.5 rounded-full bg-white shadow transition data-[state=checked]:translate-x-[18px]" />
  </SwitchRoot>
</template>
