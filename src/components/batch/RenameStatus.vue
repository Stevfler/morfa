<script setup lang="ts">
import Icon from '../Icon.vue'
import { t } from '../../lib/i18n'

defineProps<{ err: string; dupes: boolean; invalid: boolean; count: number; total: number }>()
</script>

<template>
  <p v-if="err" class="flex items-center gap-1.5 text-[12px] text-danger"><Icon name="warning" class="size-4" />{{ t('batch.invalidPattern') }}</p>
  <p v-else-if="dupes" class="flex items-center gap-1.5 text-[12px] text-warn"><Icon name="warning" class="size-4" />{{ t('batch.dupes') }}</p>
  <p v-else-if="invalid" class="flex items-center gap-1.5 text-[12px] text-warn"><Icon name="warning" class="size-4" />{{ t('batch.invalid') }}</p>
  <p v-else class="text-[12px] text-faint">{{ t('batch.willChange', { count, total }) }}</p>
</template>
