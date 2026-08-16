/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import type { DiscountCode } from './types'

export const DISCOUNT_CODE_ENABLED_STATUS = 1

export type DiscountCodeAvailability =
  | 'active'
  | 'disabled'
  | 'not_started'
  | 'expired'

export function getDiscountCodeAvailability(
  code: Pick<DiscountCode, 'status' | 'starts_time' | 'expired_time'>,
  now = Math.floor(Date.now() / 1000)
): DiscountCodeAvailability {
  if (code.status !== DISCOUNT_CODE_ENABLED_STATUS) return 'disabled'
  if (code.starts_time > now) return 'not_started'
  if (code.expired_time > 0 && code.expired_time < now) return 'expired'
  return 'active'
}
