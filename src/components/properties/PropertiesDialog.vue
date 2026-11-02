<script setup lang="ts">
import { computed } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogClose } from 'reka-ui'
import Icon from '../Icon.vue'
import PropsSingle from './PropsSingle.vue'
import PropsMulti from './PropsMulti.vue'
import { t } from '../../lib/i18n'
import * as S from '../../lib/store'

const state = S.state
const open = computed({ get: () => !!state.properties, set: (v) => !v && (state.properties = null) })
const paths = computed(() => state.properties ?? [])
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-overlay backdrop-blur-[2px]" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[86vh] w-[min(560px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-y-auto rounded-xl bg-panel ring-1 ring-fg/5 shadow-2xl outline-none"
      >
        <DialogTitle class="sr-only">{{ t('props.title') }}</DialogTitle>
        <DialogDescription class="sr-only">{{ t('props.desc') }}</DialogDescription>
        <DialogClose class="absolute top-3 right-3 rounded-md p-1.5 text-faint hover:bg-raised hover:text-fg" :aria-label="t('common.close')">
          <Icon name="x" class="size-4" />
        </DialogClose>
        <PropsSingle v-if="paths.length === 1" :path="paths[0]" />
        <PropsMulti v-else-if="paths.length" :paths="paths" />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
