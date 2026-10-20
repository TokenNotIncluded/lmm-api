/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { z } from 'zod'

import { getDrawingStorage } from './drawing-task-state'

const identity = z.string().min(1).max(256)
const digest = z.string().regex(/^[a-f0-9]{64}$/)
const operationSchema = z.object({
  requestId: z.uuid(),
  userId: z.number().int().positive(),
  toolId: identity,
  versionId: identity,
  grantId: identity,
  callId: identity.optional(),
  requestState: digest.optional(),
  confirmationURL: z.string().max(2048).optional(),
  confirmationExpiresAt: z.number().int().nonnegative().optional(),
  mayContinue: z.boolean().optional(),
  prompt: z.string().max(2000),
  model: identity,
  group: identity,
  size: z.string().max(64).optional(),
  quality: z.string().max(64).optional(),
  count: z.number().int().min(1).max(4),
  referenceCount: z.number().int().min(0).max(8),
  createdAt: z.number().int().nonnegative(),
})

export type DrawingOperation = z.infer<typeof operationSchema>

const storageKey = (userId: number) => `lmm_drawing_operation_v1_${userId}`

// Save before dispatch. No credentials or uploaded bytes are stored here.
export function saveDrawingOperation(operation: DrawingOperation): void {
  getDrawingStorage().setItem(
    storageKey(operation.userId),
    JSON.stringify(operationSchema.parse(operation))
  )
}

export function readDrawingOperation(userId: number): DrawingOperation | null {
  try {
    const raw = getDrawingStorage().getItem(storageKey(userId))
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    const result = operationSchema.safeParse(parsed)
    return result.success && result.data.userId === userId ? result.data : null
  } catch {
    return null
  }
}

// Only the owner of the same request may clear its local recovery record.
export function clearDrawingOperation(userId: number, requestId: string): void {
  if (readDrawingOperation(userId)?.requestId === requestId) {
    getDrawingStorage().removeItem(storageKey(userId))
  }
}

export const imageCapabilitySchema = z.object({
  model: identity,
  group: identity,
  endpoint: z.enum(['generations', 'edits']),
  size: z.string().min(1).max(64),
  quality: z.string().min(1).max(64),
  max_n: z.number().int().min(1).max(4),
  credits_per_image: z.number().int().nonnegative().safe(),
  max_reference_images: z.number().int().min(0).max(8),
  max_reference_bytes: z.number().int().min(0).max(10485760),
  max_total_reference_bytes: z.number().int().min(0).max(16777216),
})
export type ImageCapability = z.infer<typeof imageCapabilitySchema>
export const imageCapabilitiesSchema = z.object({
  models: z.array(imageCapabilitySchema).max(4096),
})

const generatedImageSchema = z.object({
  url: z.url().optional(),
  b64_json: z
    .string()
    .max(32 * 1024 * 1024)
    .optional(),
  revised_prompt: z.string().max(16384).optional(),
})
export const settledImageSchema = z.object({
  operation_id: identity,
  state: z.literal('completed'),
  charged_credits: z.number().int().nonnegative().safe(),
  billing_pending: z.literal(false),
  output: z.object({ data: z.array(generatedImageSchema).min(1).max(4) }),
})

export function imageConfirmationURL(
  value: unknown,
  origin: string
): string | null {
  const parsed = z
    .object({ confirmation_url: z.string().max(2048) })
    .safeParse(value)
  if (!parsed.success) return null
  try {
    const url = new URL(parsed.data.confirmation_url, origin)
    // Only the Core browser page owns approval; never navigate to provider URLs.
    if (
      url.origin !== origin ||
      !/^\/api\/user\/auth\/images\/confirm\/image:[a-f0-9]{64}$/.test(
        url.pathname
      ) ||
      // URL parsing normalizes dot segments, so also check the supplied path.
      !/^(?:https?:\/\/[^/?#\\\s@]+)?\/api\/user\/auth\/images\/confirm\/image:[a-f0-9]{64}$/.test(
        parsed.data.confirmation_url
      ) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      return null
    }
    return url.href
  } catch {
    return null
  }
}

export async function referenceImageMetadata(files: readonly File[]) {
  if (
    files.length > 8 ||
    files.reduce((sum, file) => sum + file.size, 0) > 16777216
  ) {
    throw new Error('Invalid reference image limits')
  }
  return Promise.all(
    files.map(async (file) => {
      if (
        !['image/png', 'image/jpeg', 'image/webp'].includes(file.type) ||
        file.size < 1 ||
        file.size > 10485760
      ) {
        throw new Error('Invalid reference image')
      }
      const hash = await crypto.subtle.digest(
        'SHA-256',
        await file.arrayBuffer()
      )
      return {
        media_type: file.type,
        byte_length: file.size,
        sha256: Array.from(new Uint8Array(hash), (byte) =>
          byte.toString(16).padStart(2, '0')
        ).join(''),
      }
    })
  )
}

export function imagePreviewExpiry(value: unknown): number | undefined {
  const result = z
    .object({
      preview: z.object({ expires_at: z.number().int().positive().safe() }),
    })
    .safeParse(value)
  return result.success ? result.data.preview.expires_at : undefined
}
