<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { parseLimit } from '../../lib/convert'
import { t } from '../../lib/i18n'

const MB = 1024 * 1024
const model = defineModel<number | null>({ required: true })
/** Text that isn't a size: the dialog blocks Convert and Save, or the limit would silently vanish. */
const invalid = defineModel<boolean>('invalid', { required: true })
const text = ref(model.value ? String(model.value / MB) : '')

function input() {
  const v = parseLimit(text.value)
  invalid.value = v === 'invalid'
  model.value = v === 'invalid' ? null : v
}
// Reset from outside (the dialog reopened).
watch(model, (v) => {
  if (v === null && !invalid.value) text.value = ''
})
onBeforeUnmount(() => (invalid.value = false))
</script>

<template>
  <div class="space-y-1.5">
    <label class="flex items-center gap-2 text-[12.5px] text-dim">
      <input
        v-model="text" inputmode="decimal" spellcheck="false" :placeholder="t('recipes.noLimit')" :aria-label="t('recipes.limitAria')"
        class="h-8 w-32 rounded-md border border-line bg-bg px-2.5 text-[12.5px] text-fg outline-none focus:border-accent/60" :class="invalid && 'border-danger'" :aria-invalid="invalid" @input="input"
      />
      {{ t('size.MB') }}
    </label>
    <p v-if="invalid" class="text-[11.5px] text-danger">{{ t('recipes.limitInvalid') }}</p>
    <p v-else class="text-[11.5px] text-faint">{{ t('recipes.limitHint') }}</p>
  </div>
</template>
