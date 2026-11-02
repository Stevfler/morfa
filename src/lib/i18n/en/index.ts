import { extra } from './extra'
import { common } from './common'
import { core } from './core'
import { shell } from './shell'
import { pane } from './pane'
import { tools } from './tools'

/** English is the source of truth: every other dictionary must provide the same keys. */
export const en = {
  ...common,
  ...extra,
  ...core,
  ...shell,
  ...pane,
  ...tools,
}
export type Messages = typeof en
