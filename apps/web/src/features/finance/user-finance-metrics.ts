/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
/**
 * User-facing spend must follow settled receipts minus refunds. `expense_micros`
 * instead represents the platform's cost of serving a user and must not be
 * presented as an amount the user paid.
 */
export function userNetRevenueMicros(
  revenueMicros: number,
  refundMicros: number
): number {
  return revenueMicros - refundMicros
}
