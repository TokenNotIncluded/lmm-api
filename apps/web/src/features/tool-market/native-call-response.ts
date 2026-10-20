/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { z } from 'zod'

import type { CallResponse } from './api'

export class MarketAPIError extends Error {
  readonly code: string

  constructor(code: string) {
    super(code)
    this.code = code
    this.name = 'MarketAPIError'
  }
}

const integer = z.number().int().safe().nonnegative()
const identifier = z.string().min(1).max(128)
const opaqueState = z.string().regex(/^[0-9a-f]{64}$/)
const apiCode = z.string().regex(/^TOOL_MARKET_[A-Z_]{1,64}$/)
const billingRule = z.object({
  metric: z.string(),
  rate_quota: integer,
  max_quantity: integer,
})
const mcpResult = z
  .object({
    content: z
      .array(z.object({ type: z.string().min(1) }).passthrough())
      .optional(),
    structuredContent: z.record(z.string(), z.unknown()).optional(),
    isError: z.boolean().optional(),
    _meta: z.record(z.string(), z.unknown()).optional(),
    requestState: z.string().optional(),
    inputRequests: z.record(z.string(), z.unknown()).optional(),
  })
  .strict()
  .refine(
    (value) =>
      value.content !== undefined || value.structuredContent !== undefined
  )
const receipt = z
  .object({
    call: z
      .object({
        id: opaqueState,
        user_id: integer.positive(),
        client_id: identifier,
        tool_id: identifier,
        version_id: identifier,
        grant_id: identifier,
        service_id: identifier.optional(),
        created_at: integer,
        resolve_by: integer,
        execution_status: z.enum([
          'reserved',
          'running',
          'unknown',
          'awaiting_confirmation',
          'succeeded',
          'failed',
          'cancelled',
        ]),
        settlement_status: z.enum(['reserved', 'settled', 'released']),
        price_quota: integer.optional(),
        fee_quota: integer.optional(),
        usage_source: z.enum(['tool_reported', 'platform_verified']).optional(),
        usage_quantities: z.record(z.string(), integer).optional(),
        input_tokens: integer.optional(),
        billing_mode: z
          .enum(['', 'fixed', 'input_tokens', 'metered'])
          .optional(),
        billing_rules: z.array(billingRule).nullable().optional(),
        input_token_price_quota: integer.optional(),
        max_input_tokens: integer.optional(),
      })
      .strict(),
    result: mcpResult.optional(),
    result_expires_at: integer.optional(),
    result_expired: z.boolean(),
    error_code: z.literal('TOOL_MARKET_EXECUTION_PENDING').optional(),
    request_state: opaqueState.optional(),
    awaiting_confirmation: z.boolean(),
  })
  .strict()
  .superRefine((value, context) => {
    if (
      value.awaiting_confirmation !==
        (value.call.execution_status === 'awaiting_confirmation') ||
      (value.awaiting_confirmation && !value.request_state) ||
      (value.result_expired && value.result !== undefined)
    ) {
      context.addIssue({ code: 'custom', message: 'Inconsistent call state' })
    }
  })
const success = z
  .object({ success: z.literal(true), message: z.string(), data: receipt })
  .strict()
const pending = z
  .object({
    success: z.literal(false),
    message: z.string(),
    code: z.literal('TOOL_MARKET_EXECUTION_PENDING'),
    data: receipt,
  })
  .strict()
const failure = z
  .object({ success: z.literal(false), message: z.string(), code: apiCode })
  .strict()

// This is the one boundary for invoke, query, resume and drawing multipart.
// A 409 pending receipt preserves its original identity; it is not success.
export function parseMarketCallResponse(
  raw: unknown,
  httpStatus: number
): CallResponse {
  if (httpStatus === 200) {
    const parsed = success.safeParse(raw)
    if (parsed.success && parsed.data.data.error_code === undefined) {
      return parsed.data.data
    }
  } else if (httpStatus === 409) {
    const parsed = pending.safeParse(raw)
    if (
      parsed.success &&
      parsed.data.data.error_code === 'TOOL_MARKET_EXECUTION_PENDING'
    ) {
      return parsed.data.data
    }
  }
  const denied = failure.safeParse(raw)
  if (
    denied.success &&
    denied.data.code !== 'TOOL_MARKET_EXECUTION_PENDING' &&
    httpStatus >= 400 &&
    httpStatus <= 599
  ) {
    throw new MarketAPIError(denied.data.code)
  }
  throw new MarketAPIError('TOOL_MARKET_INVALID_RESPONSE')
}

export function marketConfirmationURL(
  result: unknown,
  origin: string
): string | undefined {
  const parsed = z
    .object({
      structuredContent: z.object({
        confirmation_url: z.string().optional(),
        receipt: z
          .object({ confirmation_url: z.string().optional() })
          .optional(),
      }),
    })
    .safeParse(result)
  if (!parsed.success) return
  const structured = parsed.data.structuredContent
  const value =
    structured.confirmation_url ?? structured.receipt?.confirmation_url
  if (!value) return
  try {
    const url = new URL(value, origin)
    if (
      url.origin === origin &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      (/^\/core\/v1\/images\/confirm\/image:[a-f0-9]{64}$/.test(url.pathname) ||
        /^\/core\/v1\/bounties\/confirm\/[a-f0-9]{64}$/.test(url.pathname) ||
        /^\/core\/v1\/wallet\/confirm\/(create|claim|cancel)\/[1-9][0-9]*$/.test(
          url.pathname
        ) ||
        /^\/api\/user\/assistant\/confirmations\/[a-f0-9]{64}$/.test(
          url.pathname
        ))
    ) {
      return url.href
    }
  } catch {
    return
  }
}
