/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import type { TFunction } from 'i18next'
import * as z from 'zod'

export const DEFAULT_IP_ACCESS_ROUTING_RULES = `# China
dip(geoip:cn) -> reject`

const MAX_RULES_LENGTH = 16 * 1024

export const createIPAccessRoutingSchema = (t: TFunction) =>
  z.object({
    IPAccessRoutingRules: z
      .string()
      .trim()
      .min(1, t('At least one routing rule is required.'))
      .refine(
        (value) => new TextEncoder().encode(value).length <= MAX_RULES_LENGTH,
        t('Routing rules cannot exceed 16384 bytes.')
      ),
  })

export type IPAccessRoutingFormValues = z.infer<
  ReturnType<typeof createIPAccessRoutingSchema>
>

export function normalizeIPAccessRoutingRules(value: string) {
  return value.replaceAll('\r\n', '\n').replaceAll('\r', '\n').trim()
}
