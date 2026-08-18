/*
Copyright (C) 2026 LIghtJUNction
*/

export function buildDiscountCodeLink(code: string, origin?: string) {
  const baseOrigin =
    origin ?? (typeof window === 'undefined' ? '' : window.location.origin)
  if (!baseOrigin) return code

  const url = new URL('/wallet', baseOrigin)
  url.searchParams.set('discount_code', code)
  return url.toString()
}
