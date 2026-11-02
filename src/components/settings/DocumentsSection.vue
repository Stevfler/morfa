<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { cancelOffice, installing, installOffice, office, progress, refreshOffice, removeOffice } from '../../lib/office'
import { fmtSize } from '../../lib/util'
import { t } from '../../lib/i18n'
import Row from './Row.vue'

onMounted(() => refreshOffice(true))

const status = computed(() => {
  const s = office.value
  if (!s) return ''
  if (s.installed) return t('office.statusInstalled', { version: s.version, size: fmtSize(s.sizeOnDisk) })
  if (s.systemOffice) return t('office.statusSystem')
  if (!s.supported) return t('office.statusUnsupported')
  return t('office.statusMissing', { size: fmtSize(s.downloadSize) })
})
const pct = computed(() => (progress.value?.total ? Math.round((progress.value.done / progress.value.total) * 100) : 0))
const btn = 'h-7 rounded-md px-3 text-[12px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent/60'
</script>

<template>
  <section class="flex flex-col gap-1">
    <h3 class="text-[12px] font-medium text-dim">{{ t('office.section') }}</h3>
    <p class="text-[11.5px] text-faint">{{ t('office.sectionDesc') }}</p>
    <Row :title="t('office.status')" :desc="status">
      <button v-if="installing != null" :class="[btn, 'bg-raised text-dim hover:text-danger']" @click="cancelOffice()">{{ t('common.cancel') }}</button>
      <button v-else-if="office?.installed" :class="[btn, 'bg-raised text-fg hover:text-danger']" @click="removeOffice()">{{ t('office.remove') }}</button>
      <button
        v-else-if="office?.supported && !office.systemOffice"
        :class="[btn, 'bg-accent text-on-accent hover:brightness-110']"
        @click="installOffice()"
      >
        {{ t('office.download') }}
      </button>
    </Row>
    <div v-if="installing != null">
      <div class="h-1.5 overflow-hidden rounded-full bg-line">
        <div class="h-full rounded-full bg-accent transition-[width]" :style="{ width: `${pct || 8}%` }" />
      </div>
      <p class="mt-1 text-[11px] text-faint">
        {{ progress ? t(`office.phase.${progress.phase}`) : t('office.downloading') }}
        <template v-if="progress?.phase === 'download' && progress.total"> · {{ fmtSize(progress.done) }} / {{ fmtSize(progress.total) }}</template>
      </p>
    </div>
    <p v-if="office?.supported && !office.installed && !office.systemOffice" class="text-[11.5px] text-faint">{{ t('office.private') }}</p>
  </section>
</template>
