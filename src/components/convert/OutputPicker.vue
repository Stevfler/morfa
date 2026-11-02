<script setup lang="ts">
import { computed, ref } from 'vue'
import * as S from '../../lib/store'
import { basename } from '../../lib/util'
import { t } from '../../lib/i18n'

const props = defineProps<{ example: string }>()
const outDir = defineModel<string | null>('outDir', { required: true })
const nameTemplate = defineModel<string>('nameTemplate', { required: true })

const other = computed(() => S.otherPane()?.path ?? null)
const customDir = ref('')
const mode = ref<'next' | 'other' | 'custom'>('next')
function set(m: typeof mode.value) {
  mode.value = m
  outDir.value = m === 'next' ? null : m === 'other' ? other.value : customDir.value.trim() || null
}
const chip = (on: boolean) => (on ? 'bg-accent-soft text-accent' : 'bg-raised/60 text-dim hover:bg-raised hover:text-fg')
</script>

<template>
  <div class="space-y-3">
    <div class="flex flex-wrap gap-2">
      <button type="button" class="pill" :class="chip(mode === 'next')" @click="set('next')">{{ t('convert.nextTo') }}</button>
      <button v-if="other" type="button" class="pill max-w-64 truncate" :class="chip(mode === 'other')" :title="other" @click="set('other')">
        {{ t('convert.otherPane', { name: basename(other) || other }) }}
      </button>
      <button type="button" class="pill" :class="chip(mode === 'custom')" @click="set('custom')">{{ t('convert.another') }}</button>
    </div>
    <input
      v-if="mode === 'custom'" v-model="customDir" :placeholder="t('convert.pathPlaceholder')" spellcheck="false" :aria-label="t('convert.outFolder')"
      class="inp w-full" @input="set('custom')"
    />
    <label class="flex flex-col gap-1 text-[11.5px] text-dim">
      <span>{{ t('convert.fileName') }} <em class="text-faint not-italic">{{ t('convert.fileNameHint') }}</em></span>
      <input v-model="nameTemplate" spellcheck="false" class="inp" />
    </label>
    <p class="truncate text-[11.5px] text-faint">{{ t('convert.example') }} <span class="text-dim">{{ props.example }}</span></p>
  </div>
</template>

<style scoped>
@reference '../../style.css';
.pill {
  @apply h-8 rounded-full px-3.5 text-[12.5px] transition outline-none focus-visible:ring-2 focus-visible:ring-accent/60;
}
.inp {
  @apply h-8 rounded-md border border-line bg-bg px-2.5 text-[12.5px] text-fg outline-none focus:border-accent/60;
}
</style>
