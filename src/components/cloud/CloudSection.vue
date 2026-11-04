<script setup lang="ts">
import { computed } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from 'reka-ui'
import Icon from '../Icon.vue'
import ConnectDialog from './ConnectDialog.vue'
import { state, pane, go, cloudPath, parseCloud, parseHostKeyChange } from '../../lib/store'
import type { CloudAccount } from '../../lib/store'
import { openWeb, removeAccount, resolveHold, syncNow } from '../../lib/cloud'
import { basename } from '../../lib/util'
import { t } from '../../lib/i18n'

const current = computed(() => parseCloud(pane().path)?.id)
function statusTitle(a: CloudAccount) {
  const s = a.status
  if (s.state === 'syncing') return s.current ? t('cloud.syncingFile', { name: basename(s.current) }) : t('cloud.syncing')
  const changed = parseHostKeyChange(s.error)
  if (changed) return t('cloud.hostKeyChanged', changed)
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
          <ContextMenuItem v-if="a.kind === 'nextcloud'" class="mi" @select="openWeb(a.id)"><Icon name="external-link" />{{ t('cloud.openWeb') }}</ContextMenuItem>
          <ContextMenuItem class="mi" @select="syncNow(a.id)"><Icon name="refresh" />{{ t('cloud.syncNow') }}</ContextMenuItem>
          <ContextMenuSeparator class="sep" />
          <ContextMenuItem class="mi danger" @select="removeAccount(a)"><Icon name="x" />{{ t('cloud.remove') }}</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenuPortal>
      <div v-for="h in a.holds.filter((x) => !x.confirmed)" :key="h.pin + h.toServer" class="mx-2 mt-1 mb-1.5 rounded-lg bg-raised/60 px-2.5 py-2 text-[11.5px] leading-snug text-dim">
        <p class="flex items-start gap-1.5 text-danger"><Icon name="alert-circle" class="mt-px size-3.5 shrink-0" />{{ t(h.toServer ? 'cloud.holdServer' : 'cloud.holdLocal', { count: h.count }) }}</p>
        <p class="mt-1 text-faint">{{ t(h.gone ? (h.toServer ? 'cloud.holdGoneLocal' : 'cloud.holdGoneServer') : 'cloud.holdMass') }}</p>
        <div class="mt-1.5 flex gap-1.5">
          <button class="h-6 rounded-md bg-accent px-2 text-on-accent hover:brightness-110" @click="resolveHold(a, h, false)">{{ t('cloud.holdRestore') }}</button>
          <button class="h-6 rounded-md px-2 text-danger hover:bg-raised" @click="resolveHold(a, h, true)">{{ t('cloud.holdDeleteOk') }}…</button>
        </div>
      </div>
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
