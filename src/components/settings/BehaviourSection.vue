<script setup lang="ts">
import { settings } from '../../lib/settings'
import { state } from '../../lib/store'
import Row from './Row.vue'
import Toggle from './Toggle.vue'
</script>

<template>
  <section class="divide-y divide-line">
    <Row title="Show hidden files" desc="Dotfiles and hidden entries">
      <Toggle v-model="state.showHidden" label="Show hidden files" />
    </Row>
    <Row title="Respect .gitignore in search" desc="Skip ignored files when searching">
      <Toggle v-model="settings.respectGitignore" label="Respect .gitignore in search" />
    </Row>
  </section>
</template>
