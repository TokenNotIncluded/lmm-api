/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useState } from 'react'

import {
  CAPABILITY_TTL_MS,
  capabilityPhase,
  type CapabilityEvidence,
} from '@/lib/runtime-capabilities'

/** Expire on a timer even when neither the network nor the page produces an event. */
export function useCapabilityFreshness<T>(evidence: CapabilityEvidence<T>) {
  const [, expire] = useState(0)
  useEffect(() => {
    const remaining = evidence.dataUpdatedAt + CAPABILITY_TTL_MS - Date.now()
    if (remaining <= 0 || evidence.dataUpdatedAt <= 0) return
    const timer = window.setTimeout(
      () => expire((value) => value + 1),
      remaining
    )
    return () => window.clearTimeout(timer)
  }, [evidence.dataUpdatedAt])
  return capabilityPhase(evidence)
}
