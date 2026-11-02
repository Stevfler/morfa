<script setup lang="ts">
import type { Component } from 'vue'
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent } from 'reka-ui'
import { prettyKeys } from '../lib/commands'

defineProps<{ icon: Component; label: string; keys?: string; active?: boolean; disabled?: boolean }>()
</script>

<template>
  <TooltipRoot>
    <TooltipTrigger as-child>
      <button
        class="grid size-7 shrink-0 place-items-center rounded-md transition disabled:pointer-events-none disabled:opacity-30"
        :class="active ? 'bg-accent-soft text-accent' : 'text-dim hover:bg-raised hover:text-fg'"
        :disabled="disabled"
        :aria-label="label"
      >
        <component :is="icon" class="size-4" />
      </button>
    </TooltipTrigger>
    <TooltipPortal>
      <TooltipContent
        :side-offset="6"
        class="z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-[12px] text-fg shadow-xl"
      >
        {{ label }}
        <span v-if="keys" class="flex gap-0.5"><kbd v-for="k in prettyKeys(keys)" :key="k">{{ k }}</kbd></span>
      </TooltipContent>
    </TooltipPortal>
  </TooltipRoot>
</template>
