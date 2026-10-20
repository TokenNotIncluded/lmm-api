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
/**
 * Zod schemas for common logs
 * This file should only contain Zod schemas and types inferred from them
 */
import { z } from 'zod'

// Usage log schema
export const usageLogSchema = z.object({
  id: z.number(),
  user_id: z.number(),
  created_at: z.number(),
  type: z.number(),
  content: z.string(),
  username: z.string().default(''),
  token_name: z.string().default(''),
  model_name: z.string().default(''),
  quota: z.number().nullable().default(null),
  quota_usd: z.string().optional(),
  quota_money_available: z.boolean().optional(),
  quota_money_basis: z.string().optional(),
  billing_basis: z.string().optional(),
  state: z.string().optional(),
  gross_settled_credits: z.number().int().safe().nonnegative().nullish(),
  refunded_credits: z.number().int().safe().nonnegative().nullish(),
  net_consumption_credits: z.number().int().safe().nonnegative().nullish(),
  pricing_dimension: z.enum(['image', 'text']).optional(),
  token_usage_available: z.boolean().optional(),
  output_image_count: z.number().int().safe().positive().nullish(),
  platform_sponsored: z.boolean().optional(),
  payer_kind: z.enum(['personal', 'team']).optional(),
  wallet_debit_supported: z.boolean().optional(),
  prompt_tokens: z.number().nullable().default(null),
  completion_tokens: z.number().nullable().default(null),
  use_time: z.number().default(0),
  is_stream: z.boolean().default(false),
  channel: z.number().default(0),
  channel_name: z.string().nullish().default(''),
  token_id: z.number().nullish().default(null),
  group: z.string().default(''),
  ip: z.string().default(''),
  other: z.string().default(''),
  request_id: z.string().default(''),
  upstream_request_id: z.string().default(''),
})

export type UsageLog = z.infer<typeof usageLogSchema>
