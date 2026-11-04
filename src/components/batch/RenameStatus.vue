<script setup lang="ts">
import Icon from '../Icon.vue'
import { t } from '../../lib/i18n'

defineProps<{ err: string; unknown: string; unknownHint: string; failed: boolean; readFailed: boolean; busy: boolean; conflicts: number; missing: number; excluded: number; count: number; total: number }>()
</script>

<template>
  <p v-if="err" class="flex items-center gap-1.5 text-[12px] text-danger"><Icon name="warning" class="size-4" />{{ t('batch.invalidPattern') }}</p>
  <p v-else-if="unknown" class="flex items-center gap-1.5 text-[12px] text-danger"><Icon name="warning" class="size-4" />{{ unknownHint ? t('batch.unknownTokenCase', { token: unknown, hint: unknownHint }) : t('batch.unknownToken', { token: unknown }) }}</p>
  <p v-else-if="failed" class="flex items-center gap-1.5 text-[12px] text-danger"><Icon name="warning" class="size-4" />{{ t('batch.checkFailed') }}</p>
  <p v-else-if="busy" class="text-[12px] text-faint">{{ t('batch.checking') }}</p>
  <p v-else-if="conflicts" class="flex items-center gap-1.5 text-[12px] text-warn"><Icon name="warning" class="size-4" />{{ t('batch.conflicts', { count: conflicts }) }}</p>
  <p v-else class="text-[12px] text-faint">
    {{ t('batch.willChange', { count, total }) }}<template v-if="missing"> · {{ t('batch.leftMissing', { count: missing }) }}</template><template v-if="excluded"> · {{ t('batch.leftExcluded', { count: excluded }) }}</template><template v-if="readFailed"> · {{ t('batch.readFailed') }}</template>
  </p>
</template>
