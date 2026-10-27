/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { isAxiosError } from 'axios'

import type { TregConnectionCheck } from './api'
import { MarketAPIError } from './native-call-response'

export const tregEndpoints = {
  team: 'https://treg.to/mcp/',
  catalog: 'https://treg.to/mcp/v2/',
} as const

export type TregSurface = keyof typeof tregEndpoints

export function tregSurface(endpoint: string): TregSurface | undefined {
  try {
    const url = new URL(endpoint)
    if (
      url.origin !== 'https://treg.to' ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      return undefined
    }
    if (url.pathname.replace(/\/$/, '') === '/mcp') return 'team'
    if (url.pathname.replace(/\/$/, '') === '/mcp/v2') return 'catalog'
  } catch {
    // An incomplete custom URL is left to the existing endpoint validation.
  }
  return undefined
}

export const tregCheckCopy: Record<TregConnectionCheck['status'], string> = {
  verified:
    'treg team credential verified. Tool discovery and permission to run tools are separate checks.',
  unauthorized:
    'treg rejected this credential (401). Use an active team managed key.',
  forbidden: 'treg refused access (403). Check this key’s team permissions.',
  network_error: 'Could not reach treg. Check the network and retry.',
  timeout: 'treg verification timed out. Retry the connection check.',
  invalid_response:
    'treg returned an invalid verification response. Authentication is not verified.',
  unavailable: 'treg verification is unavailable. Retry later.',
  team_required:
    'Use a treg team managed key; this credential does not identify a team.',
}

export function tregCheckFailure(error: unknown): string {
  if (error instanceof MarketAPIError) {
    if (error.code === 'TOOL_MARKET_CONNECTION_RESPONSE') {
      return tregCheckCopy.invalid_response
    }
    if (error.code === 'TOOL_MARKET_DENIED') {
      return 'Your LMM account cannot manage tool connections.'
    }
    if (error.code === 'TOOL_MARKET_INVALID_INPUT') {
      return 'Check the official treg endpoint and team managed key.'
    }
  }
  if (isAxiosError(error)) {
    if (error.response?.status === 401) {
      return 'Sign in to LMM again before checking this connection.'
    }
    if (error.response?.status === 403) {
      return 'Your LMM account cannot manage tool connections.'
    }
    if (error.code === 'ECONNABORTED' || error.code === 'ETIMEDOUT') {
      return 'The connection check timed out. Retry to obtain a current result.'
    }
    if (!error.response) {
      return 'Could not reach LMM for the connection check. Retry.'
    }
  }
  return 'The connection check is unavailable. Authentication is not verified.'
}

export function tregDiscoveryFailure(error: unknown): string | undefined {
  if (!(error instanceof MarketAPIError)) return undefined
  switch (error.code) {
    case 'TOOL_MARKET_REMOTE_UNAUTHORIZED':
      return 'treg MCP rejected this credential (401). Verify the key and selected MCP surface.'
    case 'TOOL_MARKET_REMOTE_FORBIDDEN':
      return 'treg MCP refused tool discovery (403). Check the key’s permissions.'
    case 'TOOL_MARKET_REMOTE_TIMEOUT':
      return 'treg tool discovery timed out. Retry discovery.'
    case 'TOOL_MARKET_REMOTE_CONNECTION':
      return 'Could not reach the treg MCP endpoint. Retry discovery.'
    default:
      return undefined
  }
}
