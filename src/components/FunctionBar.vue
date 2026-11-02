<script setup lang="ts">
import { computed } from 'vue'
import type { IconName } from '../lib/icons'
import Icon from './Icon.vue'
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent } from 'reka-ui'
import * as S from '../lib/store'
import { cmd, prettyKeys } from '../lib/commands'
import { canQuickLook, keyLabel } from '../lib/util'

// Total Commander-style function-key bar, shown while the tab is split.
type Btn = { id: string; fkey: string; text: string; icon: IconName; tip: string; disabled: string | null }

const sel = computed(() => S.targets().length)
const none = computed(() => (sel.value ? null : 'Nothing selected'))
const buttons = computed<Btn[]>(() => {
  const p = S.pane()
  const other = S.otherPane()
  const n = sel.value
  const what = n === 1 ? `“${S.targets()[0].name}”` : `${n} items`
  const dest = other ? other.path.split(/[\\/]/).filter(Boolean).pop() || other.path : ''
  const xfer = S.toOtherBlocked()
  return [
    { id: 'rename', fkey: 'F2', text: 'Rename', icon: 'pencil', tip: 'Rename', disabled: none.value },
    { id: 'quicklook', fkey: 'F3', text: 'View', icon: 'quicklook', tip: 'Quick Look (or hold Space)', disabled: canQuickLook(S.visible(p)[p.cursor]) ? null : 'Nothing to preview' },
    { id: 'openapp', fkey: 'F4', text: 'Open', icon: 'external-link', tip: 'Open with default app', disabled: none.value },
    { id: 'copyother', fkey: 'F5', text: 'Copy', icon: 'copy', tip: `Copy ${what} → ${dest}`, disabled: xfer },
    { id: 'moveother', fkey: 'F6', text: 'Move', icon: 'arrow-swap', tip: `Move ${what} → ${dest}`, disabled: xfer },
    { id: 'newdir', fkey: 'F7', text: 'New folder', icon: 'folder-plus', tip: 'New folder', disabled: null },
    { id: 'trash', fkey: 'F8', text: 'Trash', icon: 'trash', tip: `Move to Trash · ${keyLabel('Shift+F8')} deletes forever`, disabled: none.value },
  ]
})

// Shift+click on Trash mirrors Shift+F8.
const run = (b: Btn, e: MouseEvent) => cmd(b.id === 'trash' && e.shiftKey ? 'delete' : b.id).run()
</script>

<template>
  <div class="flex h-9 shrink-0 items-stretch gap-1 border-t border-line bg-panel px-2 py-1">
    <TooltipRoot v-for="b in buttons" :key="b.id">
      <TooltipTrigger as-child>
        <!-- Wrapper span keeps the tooltip alive on disabled buttons (they swallow pointer events). -->
        <span class="flex min-w-0 flex-1">
          <button
            class="flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md px-2 text-[12px] text-dim transition hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-35"
            :disabled="!!b.disabled"
            :aria-label="`${b.text} (${b.fkey})`"
            @click="run(b, $event)"
          >
            <kbd class="shrink-0">{{ b.fkey }}</kbd>
            <Icon :name="b.icon" class="size-3.5 shrink-0" />
            <span class="truncate">{{ b.text }}</span>
          </button>
        </span>
      </TooltipTrigger>
      <TooltipPortal>
        <TooltipContent
          side="top"
          :side-offset="6"
          class="z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-[12px] text-fg shadow-xl"
        >
          {{ b.disabled ?? b.tip }}
          <span v-if="!b.disabled" class="flex gap-0.5"><kbd v-for="k in prettyKeys(b.fkey)" :key="k">{{ k }}</kbd></span>
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </div>
</template>
