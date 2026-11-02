<script setup lang="ts">
import { ref } from 'vue'
import Icon from './Icon.vue'
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
import DrivesView from './drives/DrivesView.vue'

const props = defineProps<{ pane: Pane; active: boolean; split: boolean }>()
const p = props.pane
const dnd = useDragDrop(p)
const menuOnItem = ref(false)
</script>

<template>
  <section class="@container flex min-h-0 flex-col" :class="split && active && 'ring-1 ring-accent/45'">
    <!-- toolbar -->
    <div class="flex h-12 shrink-0 items-center gap-1 px-3 transition" :class="split && active && 'bg-accent-soft/30'">
      <IconButton icon="chevron-left" label="Back" keys="Alt+←" :disabled="!p.back.length" @click="S.back(p)" />
      <IconButton icon="chevron-right" label="Forward" keys="Alt+→" :disabled="!p.fwd.length" @click="S.forward(p)" />
      <IconButton icon="arrow-up" label="Parent folder" keys="Backspace" :disabled="S.isSpecial(p.path) || !parent(p.path)" @click="S.up(p)" />
      <PathBar :pane="p" :active="active" :dnd="dnd" />
      <FilterBox :pane="p" />
      <IconButton icon="folder-plus" label="New folder" keys="Ctrl+Shift+N" @click="S.create('dir', p)" />
      <IconButton icon="gauge" label="Show folder sizes" keys="Ctrl+Shift+S" @click="S.calcFolderSizes(p)" />
    </div>

    <DrivesView v-if="p.path === S.DRIVES" :pane="p" />
    <template v-else>
      <ColumnHeader :pane="p" />

      <PaneMenu v-model:on-item="menuOnItem" :pane="p">
        <FileList :pane="p" :active="active" :dnd="dnd" @item-menu="menuOnItem = true" />
      </PaneMenu>

      <StatusBar :pane="p" :split="split" />
    </template>
  </section>
</template>
