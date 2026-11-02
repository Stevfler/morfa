<script setup lang="ts">
import { ref } from 'vue'
import Icon from '../Icon.vue'

const props = defineProps<{ label: string; copy?: string; mono?: boolean }>()
const copied = ref(false)
async function doCopy() {
  await navigator.clipboard.writeText(props.copy!)
  copied.value = true
  setTimeout(() => (copied.value = false), 1200)
}
</script>

<template>
  <div class="grid grid-cols-[112px_1fr_auto] items-start gap-3 px-3 py-1.5 text-[12.5px]">
    <span class="text-faint">{{ label }}</span>
    <span class="min-w-0 break-all select-text" :class="mono && 'font-mono text-[12px]'"><slot /></span>
    <button v-if="copy" class="-my-0.5 rounded-md p-1 text-faint hover:bg-raised hover:text-fg" :title="copied ? 'Copied' : 'Copy'" @click="doCopy">
      <Icon :name="copied ? 'check' : 'copy'" class="size-3.5" />
    </button>
  </div>
</template>
