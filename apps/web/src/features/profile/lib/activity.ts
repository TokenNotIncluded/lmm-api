/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import type { QuotaDataItem } from '@/features/dashboard/types'

export const PROFILE_ACTIVITY_WEEKS = 53
export const PROFILE_ACTIVITY_DAYS = PROFILE_ACTIVITY_WEEKS * 7
export const PROFILE_USAGE_WINDOW_SECONDS = 28 * 24 * 60 * 60

export type ProfileActivityView = 'daily' | 'weekly' | 'cumulative'

export type ProfileUsageRow = QuotaDataItem

export type UsageTokenCountScope = 'settled_text_requests' | null

export interface ProfileUsageQueryRange {
  start_timestamp: number
  end_timestamp: number
}

export interface ProfileActivityRange extends ProfileUsageQueryRange {
  startDate: Date
  endDate: Date
}

export interface ProfileDailyUsage {
  date: Date
  dateKey: string
  tokens: number | null
  requests: number
  outputImages: number
  sponsoredRequests: number
  tokenCountScope: UsageTokenCountScope
}

export interface ProfileUsageSummary {
  totalTokens: number | null
  peakDailyTokens: number | null
  totalImages: number
  sponsoredRequests: number
  tokenCountScope: UsageTokenCountScope
  activeDays: number
  currentStreak: number
  currentStreakCapped: boolean
  longestStreak: number
}

export interface ProfileActivityCell extends ProfileDailyUsage {
  displayTokens: number | null
  displayRequests: number
  displayImages: number
  displaySponsoredRequests: number
  level: number
  periodStart: Date
  periodEnd: Date
}

function startOfLocalDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate())
}

function addLocalDays(date: Date, amount: number): Date {
  const next = new Date(date)
  next.setDate(next.getDate() + amount)
  return next
}

export function nonNegativeUsageNumber(
  value: number | null | undefined
): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.max(0, value)
    : 0
}

/** Null means the quantity was not recorded, while numeric zero is real usage. */
export function sumKnownTextTokens(values: (number | null)[]): number | null {
  const known = values.filter((value) => value !== null)
  return values.length > 0 && known.length === 0
    ? null
    : known.reduce((sum, value) => sum + value, 0)
}

export function sumUsageCredits(values: (number | null)[]): number | null {
  return values.includes(null)
    ? null
    : values.reduce<number>((sum, value) => sum + (value ?? 0), 0)
}

export function formatProfileDateKey(date: Date): string {
  const year = String(date.getFullYear())
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

export function getProfileActivityRange(
  now = new Date()
): ProfileActivityRange {
  const endDate = startOfLocalDay(now)
  const startDate = addLocalDays(endDate, -(PROFILE_ACTIVITY_DAYS - 1))

  return {
    startDate,
    endDate,
    start_timestamp: Math.floor(startDate.getTime() / 1000),
    end_timestamp: Math.floor(now.getTime() / 1000),
  }
}

export function buildProfileUsageQueryRanges(
  startTimestamp: number,
  endTimestamp: number,
  maxWindowSeconds = PROFILE_USAGE_WINDOW_SECONDS
): ProfileUsageQueryRange[] {
  if (
    !Number.isFinite(startTimestamp) ||
    !Number.isFinite(endTimestamp) ||
    !Number.isFinite(maxWindowSeconds) ||
    startTimestamp > endTimestamp ||
    maxWindowSeconds < 1
  ) {
    return []
  }

  const ranges: ProfileUsageQueryRange[] = []
  let cursor = Math.floor(startTimestamp)
  const finalTimestamp = Math.floor(endTimestamp)
  const windowSeconds = Math.floor(maxWindowSeconds)

  while (cursor <= finalTimestamp) {
    const rangeEnd = Math.min(finalTimestamp, cursor + windowSeconds)
    ranges.push({ start_timestamp: cursor, end_timestamp: rangeEnd })
    cursor = rangeEnd + 1
  }

  return ranges
}

export function buildProfileDailyUsage(
  rows: ProfileUsageRow[],
  range: Pick<ProfileActivityRange, 'startDate' | 'endDate'>
): ProfileDailyUsage[] {
  const byDate = new Map<string, ProfileDailyUsage>()

  for (
    let date = startOfLocalDay(range.startDate);
    date <= range.endDate;
    date = addLocalDays(date, 1)
  ) {
    const dateKey = formatProfileDateKey(date)
    byDate.set(dateKey, {
      date: new Date(date),
      dateKey,
      tokens: 0,
      requests: 0,
      outputImages: 0,
      sponsoredRequests: 0,
      tokenCountScope: null,
    })
  }

  const daysWithTextTokens = new Set<string>()

  for (const row of rows) {
    const timestamp = Number(row.created_at)
    if (!Number.isFinite(timestamp)) continue

    const date = new Date(timestamp * 1000)
    const day = byDate.get(formatProfileDateKey(date))
    if (!day) continue

    if (row.token_used === null) {
      if (!daysWithTextTokens.has(day.dateKey)) day.tokens = null
    } else {
      daysWithTextTokens.add(day.dateKey)
      day.tokens = (day.tokens ?? 0) + nonNegativeUsageNumber(row.token_used)
    }
    day.requests += nonNegativeUsageNumber(row.count)
    day.outputImages += nonNegativeUsageNumber(row.output_image_count)
    if (row.platform_sponsored === true) {
      day.sponsoredRequests += nonNegativeUsageNumber(row.count)
    }
    if (row.token_count_scope === 'settled_text_requests') {
      day.tokenCountScope = row.token_count_scope
    }
  }

  return [...byDate.values()]
}

export function buildProfileUsageSummary(
  days: ProfileDailyUsage[]
): ProfileUsageSummary {
  const usageDays = days.filter(
    (day) =>
      day.tokens === null ||
      (day.tokens ?? 0) > 0 ||
      day.requests > 0 ||
      day.outputImages > 0
  )
  const totalTokens = sumKnownTextTokens(usageDays.map((day) => day.tokens))
  const peakDailyTokens =
    totalTokens === null
      ? null
      : Math.max(0, ...days.map((day) => day.tokens ?? 0))
  let activeDays = 0
  let longestStreak = 0
  let runningStreak = 0

  for (const day of days) {
    if ((day.tokens ?? 0) > 0) {
      activeDays += 1
      runningStreak += 1
      longestStreak = Math.max(longestStreak, runningStreak)
    } else {
      runningStreak = 0
    }
  }

  let cursor = days.length - 1
  if (cursor >= 0 && (days[cursor].tokens ?? 0) <= 0) cursor -= 1

  let currentStreak = 0
  while (cursor >= 0 && (days[cursor].tokens ?? 0) > 0) {
    currentStreak += 1
    cursor -= 1
  }

  return {
    totalTokens,
    peakDailyTokens,
    totalImages: days.reduce((sum, day) => sum + day.outputImages, 0),
    sponsoredRequests: days.reduce(
      (sum, day) => sum + day.sponsoredRequests,
      0
    ),
    tokenCountScope: days.some(
      (day) => day.tokenCountScope === 'settled_text_requests'
    )
      ? 'settled_text_requests'
      : null,
    activeDays,
    currentStreak,
    currentStreakCapped: currentStreak > 0 && cursor < 0,
    longestStreak,
  }
}

export function getProfileActivityLevel(value: number, max: number): number {
  if (value <= 0 || max <= 0) return 0
  return Math.min(5, Math.max(1, Math.ceil((value / max) * 5)))
}

export function buildProfileActivityCells(
  days: ProfileDailyUsage[],
  view: ProfileActivityView
): ProfileActivityCell[] {
  if (view === 'weekly') {
    const weekTotals = Array.from(
      { length: PROFILE_ACTIVITY_WEEKS },
      (_, i) => {
        const week = days.slice(i * 7, i * 7 + 7)
        return {
          tokens: sumKnownTextTokens(
            week
              .filter(
                (day) =>
                  day.tokens === null ||
                  day.requests > 0 ||
                  (day.tokens ?? 0) > 0
              )
              .map((day) => day.tokens)
          ),
          images: week.reduce((sum, day) => sum + day.outputImages, 0),
          sponsoredRequests: week.reduce(
            (sum, day) => sum + day.sponsoredRequests,
            0
          ),
          tokenCountScope: week.some(
            (day) => day.tokenCountScope === 'settled_text_requests'
          )
            ? 'settled_text_requests'
            : null,
          requests: week.reduce((sum, day) => sum + day.requests, 0),
        }
      }
    )
    const maxWeek = Math.max(0, ...weekTotals.map((week) => week.tokens ?? 0))

    return days.map((day, index) => {
      const weekIndex = Math.floor(index / 7)
      const week = weekTotals[weekIndex] ?? {
        tokens: 0,
        requests: 0,
        images: 0,
        sponsoredRequests: 0,
        tokenCountScope: null,
      }
      const filledRows =
        (week.tokens ?? 0) > 0 && maxWeek > 0
          ? Math.max(1, Math.round(((week.tokens ?? 0) / maxWeek) * 7))
          : 0
      const rowIndex = index % 7
      const periodStart = days[weekIndex * 7]?.date ?? day.date
      const periodEnd = days[weekIndex * 7 + 6]?.date ?? day.date

      return {
        ...day,
        displayTokens: week.tokens,
        displayRequests: week.requests,
        displayImages: week.images,
        displaySponsoredRequests: week.sponsoredRequests,
        tokenCountScope:
          week.tokenCountScope === 'settled_text_requests'
            ? week.tokenCountScope
            : null,
        level: rowIndex >= 7 - filledRows ? 5 : 0,
        periodStart,
        periodEnd,
      }
    })
  }

  if (view === 'cumulative') {
    const totalTokens = days.reduce((sum, day) => sum + (day.tokens ?? 0), 0)
    let runningTokens = 0
    let runningRequests = 0
    let runningImages = 0
    let runningSponsoredRequests = 0
    let hasTextTokens = false
    let hasImageTokens = false
    let tokenCountScope: UsageTokenCountScope = null

    return days.map((day) => {
      runningTokens += day.tokens ?? 0
      runningRequests += day.requests
      runningImages += day.outputImages
      runningSponsoredRequests += day.sponsoredRequests
      if (day.tokens === null) hasImageTokens = true
      else if (day.requests > 0 || day.tokens > 0) hasTextTokens = true
      if (day.tokenCountScope === 'settled_text_requests') {
        tokenCountScope = day.tokenCountScope
      }
      return {
        ...day,
        displayTokens: hasImageTokens && !hasTextTokens ? null : runningTokens,
        displayRequests: runningRequests,
        displayImages: runningImages,
        displaySponsoredRequests: runningSponsoredRequests,
        tokenCountScope,
        level: getProfileActivityLevel(runningTokens, totalTokens),
        periodStart: day.date,
        periodEnd: day.date,
      }
    })
  }

  const maxDay = Math.max(0, ...days.map((day) => day.tokens ?? 0))
  return days.map((day) => ({
    ...day,
    displayTokens: day.tokens,
    displayRequests: day.requests,
    displayImages: day.outputImages,
    displaySponsoredRequests: day.sponsoredRequests,
    level: getProfileActivityLevel(day.tokens ?? 0, maxDay),
    periodStart: day.date,
    periodEnd: day.date,
  }))
}
