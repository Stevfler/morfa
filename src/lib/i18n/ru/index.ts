import type { Dict } from '..'
import { common } from './common'
import { core } from './core'
import { shell } from './shell'
import { pane } from './pane'
import { tools } from './tools'

export const ru: Dict = {
  ...common,
  ...core,
  ...shell,
  ...pane,
  ...tools,
}
