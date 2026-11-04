<script setup lang="ts">
import Icon from '../Icon.vue'
import { isNotResponding, load_, parseHostKeyChange, type Pane } from '../../lib/store'
import { basename, keyLabel } from '../../lib/util'
import { t } from '../../lib/i18n'

defineProps<{ pane: Pane }>()
</script>

<template>
  <!-- empty states that say what to do next -->
  <div class="pointer-events-none absolute inset-0 grid place-items-center text-center">
    <div class="max-w-72 text-dim">
      <template v-if="pane.error">
        <p class="text-danger">{{ isNotResponding(pane.error) ? t('pane.notResponding') : t('pane.cantOpen') }}</p>
        <p class="mt-1 text-[12px] text-faint">{{ isNotResponding(pane.error) ? t('pane.notRespondingHint') : parseHostKeyChange(pane.error) ? t('cloud.hostKeyChanged', parseHostKeyChange(pane.error)!) : pane.error }}</p>
        <button class="pointer-events-auto mt-3 inline-flex h-7 items-center gap-1.5 rounded-md bg-raised px-2.5 text-[12px] text-fg transition hover:bg-accent-soft/40" @click="load_(pane)">
          <Icon name="refresh" class="size-3.5" />{{ t('pane.retry') }}
        </button>
      </template>
      <template v-else-if="pane.search && !pane.search.query">
        <Icon name="search" class="mx-auto mb-2 size-8 text-accent/60" />
        <p>{{ t('pane.searchInside') }} <b class="text-fg">{{ basename(pane.path) || '/' }}</b></p>
        <p class="mt-1 text-[12px] text-faint">{{ t('pane.searchHintA') }} <kbd>*.pdf</kbd> {{ t('pane.searchHintB') }}</p>
      </template>
      <template v-else-if="pane.search">
        <p>{{ pane.search.done ? t('pane.nothingFound') : t('pane.searching') }}</p>
        <p class="mt-1 text-[12px] text-faint">{{ t('pane.checked', { count: pane.search.scanned, n: pane.search.scanned.toLocaleString() }) }}</p>
      </template>
      <template v-else-if="pane.filter">
        <p>{{ t('pane.noMatch', { filter: pane.filter }) }}</p>
        <p class="mt-1 text-[12px] text-faint">{{ t('pane.pressA') }} <kbd>{{ keyLabel('Ctrl+F') }}</kbd> {{ t('pane.searchSubB') }}</p>
      </template>
      <template v-else>
        <p>{{ t('pane.empty') }}</p>
        <p class="mt-1 text-[12px] text-faint">{{ t('pane.emptyHintA') }} <kbd>{{ keyLabel('Ctrl+Shift+N') }}</kbd> {{ t('pane.emptyHintB') }}</p>
      </template>
    </div>
  </div>
</template>
