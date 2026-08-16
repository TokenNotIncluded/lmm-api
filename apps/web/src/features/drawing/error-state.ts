/*
Copyright (C) 2026 LIghtJUNction
*/

export type DrawingRequestErrorKind =
  | 'unauthenticated'
  | 'forbidden'
  | 'unavailable'
  | 'network'
  | 'http'

export function getDrawingRequestStatus(error: unknown): number | null {
  if (typeof error !== 'object' || error === null) return null
  const response = (error as { response?: unknown }).response
  if (typeof response !== 'object' || response === null) return null
  const status = (response as { status?: unknown }).status
  return typeof status === 'number' && Number.isInteger(status) ? status : null
}

export function getDrawingRequestErrorKind(
  error: unknown
): DrawingRequestErrorKind {
  const status = getDrawingRequestStatus(error)
  if (status === 401) return 'unauthenticated'
  if (status === 403) return 'forbidden'
  if (status !== null && status >= 500) return 'unavailable'
  if (status === null) return 'network'
  return 'http'
}
