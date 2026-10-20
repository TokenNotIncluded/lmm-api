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
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  buildProfileActivityCells,
  buildProfileDailyUsage,
  buildProfileUsageQueryRanges,
  buildProfileUsageSummary,
  formatProfileDateKey,
  getProfileActivityRange,
  PROFILE_ACTIVITY_DAYS,
} from './activity'

function timestamp(
  year: number,
  month: number,
  day: number,
  hour = 12
): number {
  return Math.floor(new Date(year, month - 1, day, hour).getTime() / 1000)
}

describe('profile activity range', () => {
  test('builds 53 complete weeks ending on the local current day', () => {
    const now = new Date(2026, 7, 28, 16, 30)
    const range = getProfileActivityRange(now)
    const days = buildProfileDailyUsage([], range)

    assert.equal(days.length, PROFILE_ACTIVITY_DAYS)
    assert.equal(days.at(0)?.dateKey, formatProfileDateKey(range.startDate))
    assert.equal(days.at(-1)?.dateKey, '2026-08-28')
    assert.equal(range.end_timestamp, Math.floor(now.getTime() / 1000))
  })

  test('splits requests into contiguous bounded windows', () => {
    const ranges = buildProfileUsageQueryRanges(100, 350, 100)

    assert.deepEqual(ranges, [
      { start_timestamp: 100, end_timestamp: 200 },
      { start_timestamp: 201, end_timestamp: 301 },
      { start_timestamp: 302, end_timestamp: 350 },
    ])
    assert.ok(
      ranges.every(
        (range) => range.end_timestamp - range.start_timestamp <= 100
      )
    )
  })
})

describe('profile activity aggregation', () => {
  const range = {
    startDate: new Date(2026, 7, 23),
    endDate: new Date(2026, 7, 28),
  }

  test('aggregates rows by local day and calculates truthful streaks', () => {
    const days = buildProfileDailyUsage(
      [
        {
          created_at: timestamp(2026, 8, 23),
          token_used: 10,
          count: 1,
        },
        {
          created_at: timestamp(2026, 8, 24),
          token_used: 20,
          count: 2,
        },
        {
          created_at: timestamp(2026, 8, 26),
          token_used: 30,
          count: 3,
        },
        {
          created_at: timestamp(2026, 8, 27),
          token_used: 40,
          count: 4,
        },
      ],
      range
    )

    assert.deepEqual(buildProfileUsageSummary(days), {
      totalTokens: 100,
      peakDailyTokens: 40,
      totalImages: 0,
      sponsoredRequests: 0,
      tokenCountScope: null,
      activeDays: 4,
      currentStreak: 2,
      currentStreakCapped: false,
      longestStreak: 2,
    })
    assert.equal(days[3]?.tokens, 30)
    assert.equal(days[3]?.requests, 3)
  })

  test('marks a current streak that reaches beyond the visible range', () => {
    const days = buildProfileDailyUsage(
      Array.from({ length: 6 }, (_, index) => ({
        created_at: timestamp(2026, 8, 23 + index),
        token_used: 1,
      })),
      range
    )

    const summary = buildProfileUsageSummary(days)
    assert.equal(summary.currentStreak, 6)
    assert.equal(summary.currentStreakCapped, true)
    assert.equal(summary.longestStreak, 6)
  })

  test('encodes daily, weekly, and cumulative views without changing data', () => {
    const days = buildProfileDailyUsage(
      [
        { created_at: timestamp(2026, 8, 23), token_used: 10 },
        { created_at: timestamp(2026, 8, 24), token_used: 30 },
      ],
      {
        startDate: new Date(2026, 7, 23),
        endDate: new Date(2026, 7, 29),
      }
    )

    const daily = buildProfileActivityCells(days, 'daily')
    const weekly = buildProfileActivityCells(days, 'weekly')
    const cumulative = buildProfileActivityCells(days, 'cumulative')

    assert.equal(daily[0]?.level, 2)
    assert.equal(daily[1]?.level, 5)
    assert.equal(weekly.at(-1)?.displayTokens, 40)
    assert.equal(weekly.filter((cell) => cell.level > 0).length, 7)
    assert.deepEqual(
      cumulative.map((cell) => cell.displayTokens),
      [10, 40, 40, 40, 40, 40, 40]
    )
  })
})

describe('native image activity quantities', () => {
  const range = {
    startDate: new Date(2026, 7, 23),
    endDate: new Date(2026, 7, 29),
  }
  test('preserves image-only null tokens across totals and all activity views', () => {
    const days = buildProfileDailyUsage(
      [
        {
          created_at: timestamp(2026, 8, 23),
          count: 1,
          token_used: null,
          output_image_count: 2,
          platform_sponsored: true,
          token_count_scope: 'settled_text_requests',
        },
      ],
      range
    )
    assert.equal(days[0]?.tokens, null)
    const summary = buildProfileUsageSummary(days)
    assert.equal(summary.totalTokens, null)
    assert.equal(summary.peakDailyTokens, null)
    assert.equal(summary.totalImages, 2)
    assert.equal(summary.sponsoredRequests, 1)
    assert.equal(summary.tokenCountScope, 'settled_text_requests')
    for (const view of ['daily', 'weekly', 'cumulative'] as const) {
      const cell = buildProfileActivityCells(days, view)[0]
      assert.equal(cell?.displayTokens, null)
      assert.equal(cell?.displayImages, 2)
      assert.equal(cell?.displaySponsoredRequests, 1)
    }
  })

  test('keeps real zero text tokens known when mixed with images in either order', () => {
    const image = {
      created_at: timestamp(2026, 8, 23),
      count: 1,
      token_used: null,
      output_image_count: 3,
      token_count_scope: 'settled_text_requests' as const,
    }
    const text = {
      created_at: timestamp(2026, 8, 23),
      count: 1,
      token_used: 0,
      token_count_scope: 'settled_text_requests' as const,
    }
    for (const rows of [
      [image, text],
      [text, image],
    ]) {
      const days = buildProfileDailyUsage(rows, range)
      assert.equal(days[0]?.tokens, 0)
      assert.equal(buildProfileUsageSummary(days).totalTokens, 0)
      assert.equal(
        buildProfileActivityCells(days, 'cumulative')[0]?.displayTokens,
        0
      )
      assert.equal(days[0]?.outputImages, 3)
    }
  })
})
