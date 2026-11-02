<script setup lang="ts">
import Icon from '../Icon.vue'
import { t } from '../../lib/i18n'
</script>

<template>
  <div class="flex gap-3 rounded-xl bg-warn/10 px-4 py-3.5 text-[12.5px]" role="status">
    <Icon name="warning" class="mt-0.5 size-4 shrink-0 text-warn" />
    <div class="space-y-1.5">
      <p class="font-medium text-fg">{{ t('convert.noEngine') }}</p>
      <p class="text-dim">{{ t('convert.noEngineDesc') }}</p>
      <code class="inline-block rounded-md bg-bg px-2 py-1 text-[12px] text-fg select-all">sudo pacman -S libvips</code>
    </div>
  </div>
</template>
