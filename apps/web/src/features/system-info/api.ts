/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { z } from 'zod'

import { api } from '@/lib/api'

import type {
  SystemInstanceDeleteResponse,
  SystemInstanceListResponse,
  SystemResourceSnapshot,
} from './types'

const nonnegativeInteger = z.number().int().nonnegative()
const nullableInteger = nonnegativeInteger
  .nullable()
  .transform((v) => v ?? undefined)
const nullableNumber = z
  .number()
  .nullable()
  .transform((v) => v ?? undefined)
const nativeProcessSchema = z.object({
  status: z.enum(['running', 'unavailable']),
  version: z
    .string()
    .nullable()
    .transform((v) => v ?? undefined),
  pid: nullableInteger,
  cpu_percent: nullableNumber,
  memory_bytes: nullableInteger,
  memory_percent: nullableNumber,
  threads: nullableInteger,
})

const nativeSnapshotSchema = z.object({
  // Core v1 includes both named processes; unavailable measurements are null.
  // @context-fold begin native-system-resource-fields
  schema_version: z.literal(1),
  observed_at_unix_ms: z.number().int(),
  sample_window_ms: nonnegativeInteger,
  system: z.object({
    logical_cpus: nonnegativeInteger,
    load_average: z.object({
      one: z.number(),
      five: z.number(),
      fifteen: z.number(),
    }),
    cpu_usage_percent: z.number(),
    memory_total_bytes: nonnegativeInteger,
    memory_available_bytes: nonnegativeInteger,
    memory_used_bytes: nonnegativeInteger,
    memory_usage_percent: z.number(),
  }),
  processes: z.object({
    'lmm-core': nativeProcessSchema,
    'lmm-extensions': nativeProcessSchema,
  }),
  // @context-fold end native-system-resource-fields
}) satisfies z.ZodType<SystemResourceSnapshot>

export async function listSystemInstances() {
  const res = await api.get<SystemInstanceListResponse>(
    '/api/system-info/instances'
  )
  return res.data
}

export async function getNativeSystemResources() {
  const res = await api.get<unknown>('/core/v1/admin/system-resources')
  const parsed = nativeSnapshotSchema.safeParse(res.data)
  if (!parsed.success) throw new Error('Invalid system resources response')
  return parsed.data
}

export async function deleteStaleSystemInstances() {
  const res = await api.delete<SystemInstanceDeleteResponse>(
    '/api/system-info/stale-instances'
  )
  return res.data
}

export async function deleteStaleSystemInstance(reporterId: string) {
  const res = await api.delete<SystemInstanceDeleteResponse>(
    `/api/system-info/instances/${encodeURIComponent(reporterId)}`
  )
  return res.data
}
