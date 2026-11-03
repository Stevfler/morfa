<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { TooltipProvider } from 'reka-ui'
import Icon from './components/Icon.vue'
import { state, init, newPane, nextId, newTab, closeTab, toggleSplit, focusPane, pane, back, forward } from './lib/store'
import { onGlobalKey, isReloadOrPrint } from './lib/keys'
import { basename, keyLabel } from './lib/util'
import Sidebar from './components/Sidebar.vue'
import FilePane from './components/FilePane.vue'
import Preview from './components/Preview.vue'
import Palette from './components/Palette.vue'
import TagsDialog from './components/tags/TagsDialog.vue'
import BatchRename from './components/BatchRename.vue'
import ConvertDialog from './components/convert/ConvertDialog.vue'
import PropertiesDialog from './components/properties/PropertiesDialog.vue'
import Toasts from './components/Toasts.vue'
import UpdateCard from './components/UpdateCard.vue'
import { scheduleUpdateCheck } from './lib/update'
import ConfirmDialog from './components/ConfirmDialog.vue'
import CleanupDialog from './components/cleanup/CleanupDialog.vue'
import CleanupWarning from './components/cleanup/CleanupWarning.vue'
import QuickLook from './components/quicklook/QuickLook.vue'
import IconButton from './components/IconButton.vue'
import FunctionBar from './components/FunctionBar.vue'
import HintList from './components/pane/HintList.vue'
import SettingsDialog from './components/settings/SettingsDialog.vue'
import { openSettings } from './lib/settings'
import { t } from './lib/i18n'

const ready = ref(false)
// Capture phase: a component's stopPropagation can't let webview reload/print through. Commands still run in onGlobalKey.
const blockReloadPrint = (e: KeyboardEvent) => {
  if (isReloadOrPrint(e)) e.preventDefault()
}

// Mouse side buttons (3 = back, 4 = forward) navigate the active pane, like a browser.
const onSideButton = (e: MouseEvent) => {
  if (e.button !== 3 && e.button !== 4) return
  e.preventDefault()
  if (e.type === 'mouseup') e.button === 3 ? back() : forward()
}

onMounted(async () => {
  window.addEventListener('keydown', blockReloadPrint, true)
  window.addEventListener('mousedown', onSideButton, true)
  window.addEventListener('mouseup', onSideButton, true)
  // A failing boot step (e.g. places on an unusual OS setup) must not leave the window blank.
  await init().catch((e) => console.error('init failed', e))
  if (!state.tabs.length) state.tabs = [{ id: nextId(), panes: [newPane(state.home)], active: 0 }]
  ready.value = true
  window.addEventListener('keydown', onGlobalKey)
  scheduleUpdateCheck()
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKey)
  window.removeEventListener('keydown', blockReloadPrint, true)
  window.removeEventListener('mousedown', onSideButton, true)
  window.removeEventListener('mouseup', onSideButton, true)
})

const tabTitle = (i: number) => {
  return state.tabs[i].panes.map((p) => basename(p.path) || '/').join(' ⇆ ')
}
</script>

<template>
  <TooltipProvider :delay-duration="400">
    <div v-if="ready" class="flex h-full flex-col">
      <!-- tab strip -->
      <div class="flex h-11 shrink-0 items-center gap-1 px-3">
        <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          <button
            v-for="(tab, i) in state.tabs"
            :key="tab.id"
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
          <IconButton icon="plus" :label="t('shell.newTab')" keys="Ctrl+T" @click="newTab()" />
        </div>
        <IconButton icon="columns" :label="t('shell.dualPane')" keys="Ctrl+\" :active="state.tabs[state.activeTab].panes.length > 1" @click="toggleSplit" />
        <IconButton icon="eye" :label="t('shell.infoPanel')" keys="Ctrl+I" :active="state.preview" @click="state.preview = !state.preview" />
        <IconButton icon="settings" :label="t('shell.settings')" keys="Ctrl+," @click="openSettings" />
        <button
          class="ml-1 flex h-7 items-center gap-2 rounded-full bg-raised px-3 text-[12px] text-dim transition hover:bg-raised/70 hover:text-fg focus-visible:ring-1 focus-visible:ring-accent/50"
          @click="state.palette = 'all'"
        >
          <Icon name="command" class="size-3.5" /> {{ t('shell.doAnything') }} <kbd>{{ keyLabel('Ctrl+K') }}</kbd>
        </button>
      </div>

      <div class="flex min-h-0 flex-1 gap-2 px-2 pb-2">
        <Sidebar />
        <div class="flex min-w-0 flex-1 gap-2">
          <FilePane
            v-for="(p, i) in state.tabs[state.activeTab].panes"
            :key="p.id"
            :pane="p"
            :active="i === state.tabs[state.activeTab].active"
            :split="state.tabs[state.activeTab].panes.length > 1"
            class="card min-w-0 flex-1 overflow-hidden rounded-xl bg-panel"
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
          <div class="flex h-7 items-center px-4 text-[11.5px] text-faint">
            <HintList :pane="pane()" :split="true" class="ml-auto" />
          </div>
          <FunctionBar />
        </div>
      </Transition>
    </div>

    <template v-if="ready">
      <Palette />
      <TagsDialog />
      <BatchRename />
      <ConvertDialog />
      <PropertiesDialog />
      <CleanupDialog />
      <CleanupWarning />
      <ConfirmDialog />
      <QuickLook />
      <SettingsDialog />
      <Toasts />
      <UpdateCard />
    </template>
  </TooltipProvider>
</template>
