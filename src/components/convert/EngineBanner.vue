<script setup lang="ts">
import { computed, onMounted } from 'vue'
import Icon from '../Icon.vue'
import type { Engine } from '../../lib/convert'
import { canDownload, installing, installOffice, office, progress, refreshOffice } from '../../lib/office'
import { fmtSize } from '../../lib/util'
import { t } from '../../lib/i18n'

const props = defineProps<{ engine: Engine }>()
onMounted(() => props.engine === 'office' && refreshOffice())

// Windows/macOS: offer the private Office module instead of a package-manager hint.
const module = computed(() => props.engine === 'office' && (canDownload() || installing.value != null))
const pct = computed(() => (progress.value?.total ? Math.round((progress.value.done / progress.value.total) * 100) : 0))
const msg = computed(() => ({
  image: { title: t('convert.noEngine'), desc: t('convert.noEngineDesc'), cmd: 'sudo pacman -S libvips' },
  office: { title: t('convert.noOffice'), desc: t('convert.noOfficeDesc'), cmd: 'sudo pacman -S libreoffice-fresh' },
  pandoc: { title: t('convert.noPandoc'), desc: t('convert.noPandocDesc'), cmd: 'sudo pacman -S pandoc-cli' },
})[props.engine])
</script>

<template>
  <div v-if="module" class="flex gap-3 rounded-xl bg-accent-soft px-4 py-3.5 text-[12.5px]" role="status">
    <Icon name="download" class="mt-0.5 size-4 shrink-0 text-accent" />
    <div class="min-w-0 flex-1 space-y-1.5">
      <p class="font-medium text-fg">{{ t('office.neededDesc', { size: fmtSize(office?.downloadSize) }) }}</p>
      <p class="text-dim">{{ t('office.private') }}</p>
      <div v-if="installing != null" class="pt-1">
        <div class="h-1.5 overflow-hidden rounded-full bg-line">
          <div class="h-full rounded-full bg-accent transition-[width]" :style="{ width: `${pct || 8}%` }" />
        </div>
        <p class="mt-1 text-[11px] text-faint">{{ progress ? t(`office.phase.${progress.phase}`) : t('office.downloading') }}<template v-if="pct"> · {{ pct }}%</template></p>
      </div>
      <button
        v-else
        class="mt-1 h-7 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent outline-none hover:brightness-110 focus-visible:ring-2 focus-visible:ring-accent/60"
        @click="installOffice()"
      >
        {{ t('office.download') }}
      </button>
    </div>
  </div>
  <div v-else class="flex gap-3 rounded-xl bg-warn/10 px-4 py-3.5 text-[12.5px]" role="status">
    <Icon name="warning" class="mt-0.5 size-4 shrink-0 text-warn" />
    <div class="space-y-1.5">
      <p class="font-medium text-fg">{{ msg.title }}</p>
      <p class="text-dim">{{ msg.desc }}</p>
      <code class="inline-block rounded-md bg-bg px-2 py-1 text-[12px] text-fg select-all">{{ msg.cmd }}</code>
    </div>
  </div>
</template>
