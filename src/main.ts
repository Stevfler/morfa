import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import { applySettings } from './lib/settings'

applySettings() // before mount: no flash of the default theme

createApp(App).mount('#app')
