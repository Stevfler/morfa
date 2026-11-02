import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import { watchEffect } from 'vue'
import { applySettings } from './lib/settings'
import { applyLocale } from './lib/i18n'

applySettings() // before mount: no flash of the default theme
watchEffect(applyLocale)

createApp(App).mount('#app')
