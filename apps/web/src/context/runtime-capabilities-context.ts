/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createContext, useContext } from 'react'

import type { RuntimeCapabilitiesSnapshot } from './runtime-capabilities-provider'

export const RuntimeCapabilitiesContext =
  createContext<RuntimeCapabilitiesSnapshot | null>(null)

// Public/standalone screens can render without the authenticated shell.
export function useRuntimeCapabilities() {
  return useContext(RuntimeCapabilitiesContext)
}
