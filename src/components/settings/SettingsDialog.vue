<script setup lang="ts">
import {
  DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose,
  TabsRoot, TabsList, TabsTrigger, TabsContent,
} from 'reka-ui'
import Icon from '../Icon.vue'
import { settingsOpen } from '../../lib/settings'
import AppearanceSection from './AppearanceSection.vue'
import IconsSection from './IconsSection.vue'
import BehaviourSection from './BehaviourSection.vue'

const tabs = [
  { id: 'appearance', label: 'Appearance', icon: 'palette' },
  { id: 'icons', label: 'Icons', icon: 'shapes' },
  { id: 'behaviour', label: 'Behaviour', icon: 'sliders' },
] as const
</script>

<template>
  <DialogRoot v-model:open="settingsOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent class="fixed top-1/2 left-1/2 z-50 flex h-[min(560px,88vh)] w-[min(720px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border border-line bg-panel shadow-2xl outline-none">
        <div class="flex h-12 shrink-0 items-center border-b border-line px-4">
          <DialogTitle class="text-[14px] font-medium">Settings</DialogTitle>
          <DialogDescription class="sr-only">Appearance, icons and behaviour. Changes apply immediately.</DialogDescription>
          <DialogClose class="ml-auto grid size-7 place-items-center rounded-md text-dim outline-none hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" aria-label="Close">
            <Icon name="x" class="size-4" />
          </DialogClose>
        </div>
        <TabsRoot default-value="appearance" orientation="vertical" class="flex min-h-0 flex-1">
          <TabsList class="flex w-44 shrink-0 flex-col gap-0.5 border-r border-line p-2">
            <TabsTrigger
              v-for="t in tabs" :key="t.id" :value="t.id"
              class="flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[12.5px] text-dim outline-none transition hover:bg-raised hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60 data-[state=active]:bg-accent-soft data-[state=active]:text-fg"
            >
              <Icon :name="t.icon" class="size-4" /> {{ t.label }}
            </TabsTrigger>
          </TabsList>
          <div class="min-w-0 flex-1 overflow-y-auto p-5">
            <TabsContent value="appearance" class="outline-none"><AppearanceSection /></TabsContent>
            <TabsContent value="icons" class="outline-none"><IconsSection /></TabsContent>
            <TabsContent value="behaviour" class="outline-none"><BehaviourSection /></TabsContent>
          </div>
        </TabsRoot>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
