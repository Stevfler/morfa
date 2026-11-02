<script setup lang="ts">
import { ref } from 'vue'
import { ChevronLeft, ChevronRight, ArrowUp, FolderPlus, Gauge } from 'lucide-vue-next'
import * as S from '../lib/store'
import type { Pane } from '../lib/store'
import { parent } from '../lib/util'
import { useDragDrop } from '../composables/useDragDrop'
import IconButton from './IconButton.vue'
import PathBar from './pane/PathBar.vue'
import FilterBox from './pane/FilterBox.vue'
import ColumnHeader from './pane/ColumnHeader.vue'
import FileList from './pane/FileList.vue'
import PaneMenu from './pane/PaneMenu.vue'
import StatusBar from './pane/StatusBar.vue'

const props = defineProps<{ pane: Pane; active: boolean; split: boolean }>()
const p = props.pane
const dnd = useDragDrop(p)
const menuOnItem = ref(false)
</script>

<template>
  <section class="flex min-h-0 flex-col" :class="active ? 'bg-bg' : 'bg-bg/60'">
    <!-- toolbar -->
    <div class="flex h-11 shrink-0 items-center gap-1 border-b border-line px-2" :class="split && active && 'shadow-[inset_0_2px_0_var(--color-accent)]'">
      <IconButton :icon="ChevronLeft" label="Back" keys="Alt+←" :disabled="!p.back.length" @click="S.back(p)" />
      <IconButton :icon="ChevronRight" label="Forward" keys="Alt+→" :disabled="!p.fwd.length" @click="S.forward(p)" />
      <IconButton :icon="ArrowUp" label="Parent folder" keys="Backspace" :disabled="!parent(p.path)" @click="S.up(p)" />
      <PathBar :pane="p" :active="active" :dnd="dnd" />
      <FilterBox :pane="p" />
      <IconButton :icon="FolderPlus" label="New folder" keys="Ctrl+Shift+N" @click="S.create('dir', p)" />
      <IconButton :icon="Gauge" label="Show folder sizes" keys="Ctrl+Shift+S" @click="S.calcFolderSizes(p)" />
    </div>

    <ColumnHeader :pane="p" />

    <PaneMenu v-model:on-item="menuOnItem" :pane="p">
      <FileList :pane="p" :active="active" :dnd="dnd" @item-menu="menuOnItem = true" />
    </PaneMenu>

    <StatusBar :pane="p" :split="split" />
  </section>
</template>
