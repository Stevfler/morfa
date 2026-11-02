<script setup lang="ts">
import { settings, HEX } from '../../lib/settings'
import { KIND_COLOR, KIND_ICON, type Kind } from '../../lib/util/kinds'
import { computed } from 'vue'
import { t } from '../../lib/i18n'
import Icon from '../Icon.vue'
import Row from './Row.vue'
import Segmented from './Segmented.vue'

const modes = computed(() => (['type', 'accent', 'mono', 'custom'] as const).map((value) => ({ value, label: t(`settings.iconColor.${value}`) })))
const swatches = ['#4c8dff', '#1fb5c9', '#2fb36d', '#e09422', '#f0703a', '#ec4f7c', '#8b6cff', '#7c86a2']
const sample = ['folder', 'image', 'code', 'text'] as const satisfies Kind[]
const setCustom = (v: string) => { if (HEX.test(v)) settings.iconCustom = v.toLowerCase() }
/** <input type=color> only accepts #rrggbb */
const long = (h: string) => (h.length === 4 ? '#' + [...h.slice(1)].map((c) => c + c).join('') : h)
</script>

<template>
  <div class="mt-5 pt-1">
    <Row :title="t('settings.iconColor')" :desc="t('settings.iconColorDesc')">
      <div class="w-72"><Segmented v-model="settings.iconColor" :options="modes" :label="t('settings.iconColor')" /></div>
    </Row>
    <div v-if="settings.iconColor === 'custom'" class="flex items-center gap-1.5 pb-2.5" role="group" :aria-label="t('settings.iconCustom')">
      <button
        v-for="c in swatches" :key="c" type="button" :aria-label="c" :aria-pressed="settings.iconCustom === c"
        class="size-6 rounded-full border-2 outline-none transition focus-visible:ring-2 focus-visible:ring-accent/60"
        :class="settings.iconCustom === c ? 'border-fg' : 'border-transparent'"
        :style="{ background: c }" @click="setCustom(c)"
      />
      <label class="ml-2 flex h-7 cursor-pointer items-center gap-2 rounded-full bg-raised px-3 text-[12px] text-dim hover:text-fg">
        <input type="color" class="size-4 cursor-pointer border-0 bg-transparent p-0" :value="long(settings.iconCustom)" @input="setCustom(($event.target as HTMLInputElement).value)" />
        <span class="font-mono">{{ settings.iconCustom }}</span>
      </label>
    </div>
    <div class="flex items-center gap-4 rounded-lg bg-raised px-3 py-2.5" :aria-label="t('settings.iconPreview')">
      <span v-for="k in sample" :key="k" class="flex items-center gap-1.5 text-[12px] text-dim">
        <Icon :name="KIND_ICON[k]" class="size-5" :style="{ color: KIND_COLOR[k] }" :tint="k === 'folder' ? 0.18 : 0" />
        {{ t(`settings.kind.${k}`) }}
      </span>
    </div>
  </div>
</template>
