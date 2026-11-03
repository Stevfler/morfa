<script setup lang="ts">
import { computed } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from 'reka-ui'
import Icon from '../Icon.vue'
import ConnectDialog from './ConnectDialog.vue'
import { state, pane, go, cloudPath, parseCloud } from '../../lib/store'
import type { CloudAccount } from '../../lib/store'
import { openWeb, removeAccount, syncNow } from '../../lib/cloud'
import { basename } from '../../lib/util'
import { t } from '../../lib/i18n'

const current = computed(() => parseCloud(pane().path)?.id)
function statusTitle(a: CloudAccount) {
  const s = a.status
  if (s.state === 'syncing') return s.current ? t('cloud.syncingFile', { name: basename(s.current) }) : t('cloud.syncing')
  if (s.state === 'error') return `${t('cloud.error')}: ${s.error ?? ''}`
  return `${a.login} · ${a.localRoot}`
}
</script>

<template>
  <section>
    <div class="flex items-center px-2 pb-1">
      <h3 class="flex-1 text-[11px] font-medium uppercase tracking-wider text-faint">{{ t('cloud.title') }}</h3>
      <button class="grid size-5 place-items-center rounded text-faint hover:bg-raised hover:text-fg" :title="t('cloud.add')" @click="state.cloudConnect = true">
        <Icon name="plus" class="size-3.5" />
      </button>
    </div>
    <button v-if="!state.cloud.length" class="mx-2 block rounded-lg bg-raised/50 px-3 py-2.5 text-left text-[11.5px] leading-snug text-faint hover:text-dim" @click="state.cloudConnect = true">
      {{ t('cloud.empty') }}
    </button>
    <ContextMenuRoot v-for="a in state.cloud" :key="a.id">
      <ContextMenuTrigger as-child>
        <button class="item" :class="current === a.id && 'on'" :title="statusTitle(a)" @click="go(pane(), cloudPath(a.id))">
          <Icon name="cloud" class="size-4 shrink-0" /><span class="truncate">{{ a.name }}</span>
          <Icon v-if="a.status.state === 'syncing'" name="loader" class="ml-auto size-3.5 shrink-0 animate-spin text-faint" />
          <Icon v-else-if="a.status.state === 'error'" name="alert-circle" class="ml-auto size-3.5 shrink-0 text-danger" />
        </button>
      </ContextMenuTrigger>
      <ContextMenuPortal>
        <ContextMenuContent class="menu">
          <ContextMenuItem class="mi" @select="go(pane(), cloudPath(a.id))"><Icon name="folder-open" />{{ t('shell.open') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="go(pane(), a.localRoot)"><Icon name="hard-drive" />{{ t('cloud.openLocal') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="openWeb(a.id)"><Icon name="external-link" />{{ t('cloud.openWeb') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="syncNow(a.id)"><Icon name="refresh" />{{ t('cloud.syncNow') }}</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi danger" @select="removeAccount(a)"><Icon name="x" />{{ t('cloud.remove') }}</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenuPortal>
    </ContextMenuRoot>
    <ConnectDialog />
  </section>
</template>

<style scoped>
@reference '../../style.css';
.item {
  @apply flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12.5px] text-dim transition hover:bg-raised hover:text-fg;
}
.item.on {
  @apply bg-accent-soft text-fg;
}
</style>
