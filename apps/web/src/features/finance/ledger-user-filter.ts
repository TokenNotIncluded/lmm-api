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

/**
 * Financial audit links must opt out of the user page's L0-only default. A
 * payment or refund can belong to any account level.
 */
export function financeLedgerUserSearch(userId?: number) {
  const filter = financeLedgerUserFilter(userId)
  if (!filter) return undefined
  return {
    page: 1,
    pageSize: undefined,
    filter,
    status: [],
    role: [],
    group: '',
    l0Only: false,
  }
}
