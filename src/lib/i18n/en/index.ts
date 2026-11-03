import { extra } from './extra'
import { common } from './common'
import { core } from './core'
import { shell } from './shell'
import { pane } from './pane'
import { tools } from './tools'
import { media } from './media'
import { cloud } from './cloud'
import { update } from './update'

/** English is the source of truth: every other dictionary must provide the same keys. */
export const en = {
  ...common,
  ...extra,
  ...core,
  ...shell,
  ...pane,
  ...tools,
  ...media,
  ...cloud,
  ...update,
}
export type Messages = typeof en
