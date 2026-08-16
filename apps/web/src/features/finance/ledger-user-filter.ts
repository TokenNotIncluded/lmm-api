/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
export function financeLedgerUserFilter(userId?: number) {
  return typeof userId === 'number' &&
    Number.isSafeInteger(userId) &&
    userId > 0
    ? String(userId)
    : ''
}
