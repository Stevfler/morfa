<script lang="ts">
export interface Run { text: string; b?: boolean; i?: boolean }
export type Block =
  | { t: 'h'; level: number; runs: Run[] }
  | { t: 'p'; runs: Run[] }
  | { t: 'li'; level: number; runs: Run[] }
  | { t: 'table'; rows: string[][] }
</script>

<script setup lang="ts">
defineProps<{ blocks: Block[] }>()
const H = ['text-[22px]', 'text-[18px]', 'text-[16px]', 'text-[14.5px]', 'text-[13.5px]', 'text-[13px]']
</script>

<template>
  <div class="text-[13.5px] leading-[1.65] text-fg">
    <template v-for="(b, k) in blocks" :key="k">
      <table v-if="b.t === 'table'" class="my-3 w-full border-collapse text-[12.5px]">
        <tr v-for="(r, i) in b.rows" :key="i">
          <td v-for="(c, j) in r" :key="j" class="border border-line px-2 py-1 align-top whitespace-pre-wrap" :class="i === 0 && 'bg-panel font-medium'">{{ c }}</td>
        </tr>
      </table>
      <component
        :is="b.t === 'h' ? 'h3' : 'p'"
        v-else
        class="whitespace-pre-wrap"
        :class="[
          b.t === 'h' ? [H[b.level - 1], 'mt-5 mb-2 font-semibold leading-snug'] : 'mb-2 min-h-[1em]',
          b.t === 'li' && 'relative pl-5 before:absolute before:left-1 before:text-dim before:content-[\'•\']',
        ]"
        :style="b.t === 'li' ? { marginLeft: `${b.level * 1.25}rem` } : undefined"
      >
        <span v-for="(r, i) in b.runs" :key="i" :class="[r.b && 'font-semibold', r.i && 'italic']">{{ r.text }}</span>
      </component>
    </template>
  </div>
</template>
