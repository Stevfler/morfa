<script setup lang="ts">
import { Search } from 'lucide-vue-next'
import type { Pane } from '../../lib/store'
import { basename } from '../../lib/util'

defineProps<{ pane: Pane }>()
</script>

<template>
  <!-- empty states that say what to do next -->
  <div class="pointer-events-none absolute inset-0 grid place-items-center text-center">
    <div class="max-w-72 text-dim">
      <template v-if="pane.error">
        <p class="text-danger">Can’t open this folder</p>
        <p class="mt-1 text-[12px] text-faint">{{ pane.error }}</p>
      </template>
      <template v-else-if="pane.search && !pane.search.query">
        <Search class="mx-auto mb-2 size-8 text-accent/60" />
        <p>Search everything inside <b class="text-fg">{{ basename(pane.path) || '/' }}</b></p>
        <p class="mt-1 text-[12px] text-faint">Words match anywhere in the name. Add <kbd>*.pdf</kbd> to filter by type.</p>
      </template>
      <template v-else-if="pane.search">
        <p>{{ pane.search.done ? 'Nothing found' : 'Searching…' }}</p>
        <p class="mt-1 text-[12px] text-faint">{{ pane.search.scanned.toLocaleString() }} items checked</p>
      </template>
      <template v-else-if="pane.filter">
        <p>No names match “{{ pane.filter }}”</p>
        <p class="mt-1 text-[12px] text-faint">Press <kbd>Ctrl F</kbd> to search subfolders too</p>
      </template>
      <template v-else>
        <p>This folder is empty</p>
        <p class="mt-1 text-[12px] text-faint">Drop files here or press <kbd>Ctrl Shift N</kbd> for a new folder</p>
      </template>
    </div>
  </div>
</template>
