<script setup lang="ts">
import { computed } from 'vue'
import type { IconName } from '../lib/icons'
import Icon from './Icon.vue'
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent } from 'reka-ui'
import * as S from '../lib/store'
import { cmd, prettyKeys } from '../lib/commands'
import { canQuickLook, keyLabel } from '../lib/util'
import { t } from '../lib/i18n'

// Total Commander-style function-key bar, shown while the tab is split.
type Btn = { id: string; fkey: string; text: string; icon: IconName; tip: string; disabled: string | null }

const sel = computed(() => S.targets().length)
const none = computed(() => (sel.value ? null : t('fbar.nothingSelected')))
const buttons = computed<Btn[]>(() => {
  const p = S.pane()
  const other = S.otherPane()
  const n = sel.value
  const what = n === 1 ? t('shell.quoted', { name: S.targets()[0].name }) : t('shell.items', { count: n })
  const dest = other ? other.path.split(/[\\/]/).filter(Boolean).pop() || other.path : ''
  const xfer = S.toOtherBlocked()
  return [
    { id: 'rename', fkey: 'F2', text: t('fbar.rename'), icon: 'pencil', tip: t('fbar.rename'), disabled: n > 1 && !S.canBatchRename() ? t('batch.notHere') : none.value },
    { id: 'quicklook', fkey: 'F3', text: t('fbar.view'), icon: 'quicklook', tip: t('fbar.quickLookTip'), disabled: canQuickLook(S.visible(p)[p.cursor]) ? null : t('fbar.nothingToPreview') },
    { id: 'openapp', fkey: 'F4', text: t('shell.open'), icon: 'external-link', tip: t('shell.openDefault'), disabled: none.value },
    { id: 'copyother', fkey: 'F5', text: t('fbar.copy'), icon: 'copy', tip: t('fbar.copyTip', { what, dest }), disabled: xfer },
    { id: 'moveother', fkey: 'F6', text: t('fbar.move'), icon: 'arrow-swap', tip: t('fbar.moveTip', { what, dest }), disabled: xfer },
    { id: 'newdir', fkey: 'F7', text: t('shell.newFolder'), icon: 'folder-plus', tip: t('shell.newFolder'), disabled: null },
    { id: 'trash', fkey: 'F8', text: t('fbar.trash'), icon: 'trash', tip: t('fbar.trashTip', { key: keyLabel('Shift+F8') }), disabled: none.value },
    { id: 'compare', fkey: 'F9', text: t('fbar.compare'), icon: 'columns', tip: t('cmd.compare'), disabled: null },
  ]
})

// Shift+click on Trash mirrors Shift+F8.
const run = (b: Btn, e: MouseEvent) => cmd(b.id === 'trash' && e.shiftKey ? 'delete' : b.id).run()
</script>

<template>
  <div class="flex h-9 shrink-0 items-stretch gap-1.5 px-2 pb-2">
    <TooltipRoot v-for="b in buttons" :key="b.id">
      <TooltipTrigger as-child>
        <!-- Wrapper span keeps the tooltip alive on disabled buttons (they swallow pointer events). -->
        <span class="flex min-w-0 flex-1">
          <button
            class="flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-lg px-2 text-[12px] text-dim transition hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-35"
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
          class="z-50 flex items-center gap-2 rounded-md bg-raised px-2 py-1 ring-1 ring-fg/5 text-[12px] text-fg shadow-xl"
        >
          {{ b.disabled ?? b.tip }}
          <span v-if="!b.disabled" class="flex gap-0.5"><kbd v-for="k in prettyKeys(b.fkey)" :key="k">{{ k }}</kbd></span>
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </div>
</template>
