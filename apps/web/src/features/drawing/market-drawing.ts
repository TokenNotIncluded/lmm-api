/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { z } from 'zod'

import { api } from '@/lib/api'

import {
  marketAPI,
  parseMarketCallResponse,
  type MarketTool,
} from '../tool-market/api'
import { referenceImageMetadata } from './native-operation'

export async function discoverDrawingTool(): Promise<MarketTool> {
  const services = await marketAPI.list('', 0, 'builtin')
  const details = await Promise.all(
    services.map((service) => marketAPI.detail(service.id))
  )
  const tools = details.flatMap((detail) =>
    detail.version.execution_type === 'builtin'
      ? detail.tools.filter(
          (tool) =>
            tool.name === 'drawing.generate' &&
            tool.version_id === detail.version.id
        )
      : []
  )
  if (tools.length !== 1 || tools[0].price_quota !== 0) {
    throw new Error('Image tool is unavailable')
  }
  return tools[0]
}

// The user grants one exact free outer call; the actual image price still needs
// Core's separate browser confirmation before any provider request.
export async function authorizeDrawingOnce(tool: MarketTool) {
  if (tool.name !== 'drawing.generate' || tool.price_quota !== 0) {
    throw new Error('Unexpected image tool pricing')
  }
  const installation = {
    client_id: 'web-market',
    tool_id: tool.tool_id,
    version_id: tool.version_id,
  }
  const grant = await marketAPI.grant({
    ...installation,
    max_price_quota: 0,
    max_total_quota: 0,
    max_calls: 1,
    expires_at: Math.floor(Date.now() / 1000) + 3600,
  })
  await marketAPI.install(installation, true)
  return grant
}

export function drawingStructuredContent(result: unknown): unknown {
  const parsed = z
    .object({
      isError: z.literal(false).optional(),
      structuredContent: z.unknown(),
    })
    .parse(result)
  if (parsed.structuredContent === undefined) {
    throw new Error('Missing image receipt')
  }
  return parsed.structuredContent
}

export async function drawingMultipartBody(
  request: {
    tool_id: string
    version_id: string
    grant_id: string
    request_id: string
    arguments: Record<string, unknown>
  },
  files: readonly File[]
): Promise<FormData> {
  const references = await referenceImageMetadata(files)
  const form = new FormData()
  form.append(
    'request',
    JSON.stringify({
      ...request,
      arguments: { ...request.arguments, reference_images: references },
    })
  )
  for (const file of files) form.append('images', file, file.name)
  return form
}

export async function postDrawingMultipart(form: FormData) {
  const response = await api.post<unknown>(
    '/api/tool-market/drawing/invoke',
    form,
    {
      timeout: 60000,
      skipBusinessError: true,
      skipErrorHandler: true,
      validateStatus: (status) => status === 200 || status === 409,
    }
  )
  return parseMarketCallResponse(response.data, response.status)
}
