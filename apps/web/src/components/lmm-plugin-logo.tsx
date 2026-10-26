/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { cn } from '@/lib/utils'

export type LmmPluginId =
  | 'pi'
  | 'dsh'
  | 'opencode'
  | 'codewhale'
  | 'vscode'
  | 'zed'

/** Decorative client mark; its adjacent text supplies the accessible name. */
export function LmmPluginLogo({
  plugin,
  className,
}: {
  plugin: LmmPluginId
  className?: string
}) {
  return (
    <img
      src={`/plugin-logos/${plugin}.svg`}
      alt=''
      aria-hidden='true'
      width={32}
      height={32}
      decoding='async'
      className={cn('size-8 shrink-0', className)}
    />
  )
}
