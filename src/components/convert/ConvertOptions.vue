<script setup lang="ts">
import QualityPicker from './QualityPicker.vue'
import ResizePicker from './ResizePicker.vue'
import type { ConvertOpts } from '../../lib/convert'
import { t } from '../../lib/i18n'

/** Quality / size / metadata sections; which ones apply is decided by the dialog. */
// `opts` is the dialog's reactive options object, edited in place.
const { opts } = defineProps<{ opts: ConvertOpts; quality: boolean; resize: boolean; strip: boolean; lossless: boolean; audio: boolean }>()
const preset = defineModel<string>('preset', { required: true })
</script>

<template>
  <section v-if="quality">
    <h3 class="sec">{{ t('convert.quality') }}</h3>
    <QualityPicker
      v-model:quality="opts.quality" v-model:preset="preset" :lossless="lossless"
      :lossless-desc="audio ? t('convert.losslessAudio') : undefined"
    />
  </section>
  <section v-if="resize">
    <h3 class="sec">{{ t('convert.size') }}</h3>
    <ResizePicker v-model="opts.resize" />
  </section>
  <section v-if="strip">
    <label class="flex cursor-pointer items-center justify-between gap-4 rounded-lg px-1 py-1">
      <span>
        <span class="block text-[13px] text-fg">{{ t('convert.strip') }}</span>
        <span class="block text-[11.5px] text-faint">{{ audio ? t('convert.stripMediaDesc') : t('convert.stripDesc') }}</span>
      </span>
      <input v-model="opts.stripMetadata" type="checkbox" role="switch" class="size-4 accent-accent" />
    </label>
  </section>
</template>

<style scoped>
@reference '../../style.css';
.sec {
  @apply mb-3 text-[11px] font-medium tracking-wider text-faint uppercase;
}
</style>
