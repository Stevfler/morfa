<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../Icon.vue'
import type { Engine } from '../../lib/convert'
import { t } from '../../lib/i18n'

const props = defineProps<{ engine: Engine }>()
const msg = computed(() => ({
  image: { title: t('convert.noEngine'), desc: t('convert.noEngineDesc'), cmd: 'sudo pacman -S libvips' },
  office: { title: t('convert.noOffice'), desc: t('convert.noOfficeDesc'), cmd: 'sudo pacman -S libreoffice-fresh' },
  pandoc: { title: t('convert.noPandoc'), desc: t('convert.noPandocDesc'), cmd: 'sudo pacman -S pandoc-cli' },
})[props.engine])
</script>

<template>
  <div class="flex gap-3 rounded-xl bg-warn/10 px-4 py-3.5 text-[12.5px]" role="status">
    <Icon name="warning" class="mt-0.5 size-4 shrink-0 text-warn" />
    <div class="space-y-1.5">
      <p class="font-medium text-fg">{{ msg.title }}</p>
      <p class="text-dim">{{ msg.desc }}</p>
      <code class="inline-block rounded-md bg-bg px-2 py-1 text-[12px] text-fg select-all">{{ msg.cmd }}</code>
    </div>
  </div>
</template>
