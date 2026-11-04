import { extra } from './extra'
import type { Dict } from '..'
import { common } from './common'
import { core } from './core'
import { shell } from './shell'
import { pane } from './pane'
import { tools } from './tools'
import { media } from './media'
import { cloud } from './cloud'
import { archive } from './archive'
import { update } from './update'
import { compare } from './compare'
import { dedupe } from './dedupe'
import { journal } from './journal'
import { autoconvert } from './autoconvert'

export const ru: Dict = {
  ...common,
  ...extra,
  ...core,
  ...shell,
  ...pane,
  ...tools,
  ...media,
  ...cloud,
  ...archive,
  ...update,
  ...compare,
  ...dedupe,
  ...journal,
  ...autoconvert,
}
