<script setup lang="ts">
import { computed } from 'vue'
import { settings, type IconPack } from '../lib/settings'
import { packs, loadPack, type IconName } from '../lib/icons'

// `tint` gives stroke packs a soft filled body (folders); duotone packs already look that way.
const props = withDefaults(
  defineProps<{ name: IconName; pack?: IconPack; strokeWidth?: number | string; tint?: number }>(),
  { strokeWidth: 2 },
)

const data = computed(() => {
  const id = props.pack ?? settings.iconPack
  const p = packs[id]
  if (!p) loadPack(id)
  return p && { stroke: p.stroke, icon: p.icons[props.name] }
})
</script>

<template>
  <!-- Always an <svg> of the same box, so nothing shifts while a pack chunk loads. -->
  <svg
    xmlns="http://www.w3.org/2000/svg"
    :viewBox="data?.icon?.[0] ?? '0 0 24 24'"
    width="1em" height="1em" aria-hidden="true"
    :fill="data?.stroke ? (tint ? 'currentColor' : 'none') : 'currentColor'"
    :fill-opacity="data?.stroke && tint ? tint : undefined"
    :stroke-width="data?.stroke ? strokeWidth : undefined"
    v-html="data?.icon?.[1] ?? ''"
  />
</template>
