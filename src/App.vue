<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { TooltipProvider } from 'reka-ui'
import Icon from './components/Icon.vue'
import { state, init, newPane, nextId, newTab, closeTab, toggleSplit, focusPane, pane } from './lib/store'
import { onGlobalKey, isReloadOrPrint } from './lib/keys'
import { basename, keyLabel } from './lib/util'
import Sidebar from './components/Sidebar.vue'
import FilePane from './components/FilePane.vue'
import Preview from './components/Preview.vue'
import Palette from './components/Palette.vue'
import BatchRename from './components/BatchRename.vue'
import ConvertDialog from './components/convert/ConvertDialog.vue'
import Toasts from './components/Toasts.vue'
import ConfirmDialog from './components/ConfirmDialog.vue'
import CleanupDialog from './components/cleanup/CleanupDialog.vue'
import CleanupWarning from './components/cleanup/CleanupWarning.vue'
import QuickLook from './components/quicklook/QuickLook.vue'
import IconButton from './components/IconButton.vue'
import FunctionBar from './components/FunctionBar.vue'
import HintList from './components/pane/HintList.vue'
import SettingsDialog from './components/settings/SettingsDialog.vue'
import { openSettings } from './lib/settings'

const ready = ref(false)
// Capture phase: a component's stopPropagation can't let webview reload/print through. Commands still run in onGlobalKey.
const blockReloadPrint = (e: KeyboardEvent) => {
  if (isReloadOrPrint(e)) e.preventDefault()
}

onMounted(async () => {
  window.addEventListener('keydown', blockReloadPrint, true)
  // A failing boot step (e.g. places on an unusual OS setup) must not leave the window blank.
  await init().catch((e) => console.error('init failed', e))
  if (!state.tabs.length) state.tabs = [{ id: nextId(), panes: [newPane(state.home)], active: 0 }]
  ready.value = true
  window.addEventListener('keydown', onGlobalKey)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKey)
  window.removeEventListener('keydown', blockReloadPrint, true)
})

const tabTitle = (i: number) => {
  const t = state.tabs[i]
  return t.panes.map((p) => basename(p.path) || '/').join(' ⇆ ')
}
</script>

<template>
  <TooltipProvider :delay-duration="400">
    <div v-if="ready" class="flex h-full flex-col">
      <!-- tab strip -->
      <div class="flex h-10 shrink-0 items-center gap-1 border-b border-line bg-panel px-2">
        <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          <button
            v-for="(t, i) in state.tabs"
            :key="t.id"
            class="group flex h-7 max-w-56 shrink-0 items-center gap-2 rounded-md px-3 text-[12.5px] transition"
            :class="i === state.activeTab ? 'bg-raised text-fg shadow-sm' : 'text-dim hover:bg-raised/60 hover:text-fg'"
            @click="state.activeTab = i"
            @mousedown.middle.prevent="closeTab(i)"
          >
            <span class="truncate">{{ tabTitle(i) }}</span>
            <Icon name="x"
              v-if="state.tabs.length > 1"
              class="size-3.5 shrink-0 opacity-0 transition group-hover:opacity-60 hover:!opacity-100"
              @click.stop="closeTab(i)"
            />
          </button>
          <IconButton icon="plus" label="New tab" keys="Ctrl+T" @click="newTab()" />
        </div>
        <IconButton icon="columns" label="Dual pane" keys="Ctrl+\" :active="state.tabs[state.activeTab].panes.length > 1" @click="toggleSplit" />
        <IconButton icon="eye" label="Info panel" keys="Ctrl+I" :active="state.preview" @click="state.preview = !state.preview" />
        <IconButton icon="settings" label="Settings" keys="Ctrl+," @click="openSettings" />
        <button
          class="ml-1 flex h-7 items-center gap-2 rounded-md border border-line bg-bg px-2.5 text-[12px] text-dim transition hover:border-accent/50 hover:text-fg"
          @click="state.palette = 'all'"
        >
          <Icon name="command" class="size-3.5" /> Do anything… <kbd>{{ keyLabel('Ctrl+K') }}</kbd>
        </button>
      </div>

      <div class="flex min-h-0 flex-1">
        <Sidebar />
        <div class="flex min-w-0 flex-1">
          <FilePane
            v-for="(p, i) in state.tabs[state.activeTab].panes"
            :key="p.id"
            :pane="p"
            :active="i === state.tabs[state.activeTab].active"
            :split="state.tabs[state.activeTab].panes.length > 1"
            class="min-w-0 flex-1"
            :class="i > 0 && 'border-l border-line'"
            @mousedown="focusPane(i)"
          />
        </div>
        <Preview v-if="state.preview" />
      </div>
      <Transition
        enter-active-class="transition-[margin,opacity] duration-150 ease-out"
        leave-active-class="transition-[margin,opacity] duration-100 ease-in"
        enter-from-class="-mb-16 opacity-0"
        leave-to-class="-mb-16 opacity-0"
      >
        <div v-if="state.tabs[state.activeTab].panes.length > 1">
          <!-- One hints strip for both panes; it follows the active one. -->
          <div class="flex h-7 items-center border-t border-line px-3 text-[11.5px] text-faint">
            <HintList :pane="pane()" :split="true" class="ml-auto" />
          </div>
          <FunctionBar />
        </div>
      </Transition>
    </div>

    <template v-if="ready">
      <Palette />
      <BatchRename />
      <ConvertDialog />
      <CleanupDialog />
      <CleanupWarning />
      <ConfirmDialog />
      <QuickLook />
      <SettingsDialog />
      <Toasts />
    </template>
  </TooltipProvider>
</template>
