/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
/**
 * Per-model usage stats for the profile share report.
 *
 * The backend groups `/api/data/self` by `model_name` and hour, so a single
 * bounded window carries everything we need — but one request may never span
 * more than a month (`controller/usedata.go` rejects a wider range), so the
 * caller fans out with `buildProfileUsageQueryRanges` and hands every window
 * here. `mergeModelUsageRows` then folds them into one row per model.
 */
import {
  buildProfileUsageQueryRanges,
  nonNegativeUsageNumber,
  sumKnownTextTokens,
  sumUsageCredits,
  type UsageTokenCountScope,
  type ProfileUsageQueryRange,
  type ProfileUsageRow,
} from './activity'

/** Mirrors the server-side window cap in `controller/usedata.go`. */
export const MODEL_USAGE_WINDOW_SECONDS = 28 * 24 * 60 * 60

/** Bounded history for the explicitly labelled one-year report. */
export const MODEL_USAGE_YEAR_DAYS = 365

export type ModelUsageRangeKey = '7d' | '30d' | '365d'

export type ModelUsageRow = ProfileUsageRow

export interface ModelUsageStat {
  modelName: string
  requests: number
  tokens: number | null
  quota: number | null
  outputImages: number
  sponsoredRequests: number
  sponsoredQuota: number | null
  tokenCountScope: UsageTokenCountScope
  /** Share of the largest model's quota, 0–1. Drives bar lengths. */
  intensity: number
  /** Share of total quota, 0–1. */
  share: number
}

export interface ModelUsageTotals {
  requests: number
  tokens: number | null
  quota: number | null
  outputImages: number
  sponsoredRequests: number
  sponsoredQuota: number | null
  tokenCountScope: UsageTokenCountScope
  modelCount: number
  /** Most-used model by quota, or null when nothing was spent. */
  topModel: ModelUsageStat | null
}

export interface ModelUsageReport {
  models: ModelUsageStat[]
  totals: ModelUsageTotals
  shareMetric: 'quota' | 'tokens' | 'requests'
}

/**
 * Users often hit the same upstream model through aliases, and the API may
 * return an empty name for legacy rows — surface those honestly rather than
 * dropping their spend on the floor.
 */
export function normalizeModelName(modelName?: string | null): string {
  const trimmed = (modelName ?? '').trim()
  return trimmed.length > 0 ? trimmed : 'unknown'
}

/**
 * Fold bounded windows into one row per model. Rows whose `model_name` is
 * missing still count, aggregated under `unknown`.
 */
export function mergeModelUsageRows(
  rows: ModelUsageRow[]
): Map<string, ModelUsageStat> {
  const byModel = new Map<string, ModelUsageStat>()

  for (const row of rows) {
    const modelName = normalizeModelName(row.model_name)
    const existing = byModel.get(modelName) ?? {
      modelName,
      requests: 0,
      tokens: null,
      quota: 0,
      outputImages: 0,
      sponsoredRequests: 0,
      sponsoredQuota: 0,
      tokenCountScope: null,
      intensity: 0,
      share: 0,
    }

    existing.requests += nonNegativeUsageNumber(row.count)
    existing.tokens = sumKnownTextTokens([
      existing.tokens,
      row.token_used === null ? null : nonNegativeUsageNumber(row.token_used),
    ])
    const rowQuota =
      row.quota === null ? null : nonNegativeUsageNumber(row.quota)
    existing.quota = sumUsageCredits([existing.quota, rowQuota])
    existing.outputImages += nonNegativeUsageNumber(row.output_image_count)
    if (row.platform_sponsored === true) {
      existing.sponsoredRequests += nonNegativeUsageNumber(row.count)
      existing.sponsoredQuota = sumUsageCredits([
        existing.sponsoredQuota,
        rowQuota,
      ])
    }
    if (row.token_count_scope === 'settled_text_requests') {
      existing.tokenCountScope = row.token_count_scope
    }

    byModel.set(modelName, existing)
  }

  return byModel
}

/**
 * Rank models by spend, then compute bar intensities so the chart and the
 * leaderboard agree on scale. Ties fall back to tokens so that a model with
 * real traffic never hides below an idle one.
 */
export function buildModelUsageReport(rows: ModelUsageRow[]): ModelUsageReport {
  const models = [...mergeModelUsageRows(rows).values()]
  const totalQuota = sumUsageCredits(models.map((model) => model.quota))
  const totalTokens = sumKnownTextTokens(models.map((model) => model.tokens))
  const totalRequests = models.reduce((sum, model) => sum + model.requests, 0)
  const weightMetric =
    totalQuota !== null && totalQuota > 0
      ? 'quota'
      : totalTokens !== null && totalTokens > 0
        ? 'tokens'
        : 'requests'
  const weightOf = (model: ModelUsageStat) => model[weightMetric] ?? 0
  models.sort(
    (a, b) =>
      weightOf(b) - weightOf(a) ||
      (b.tokens ?? 0) - (a.tokens ?? 0) ||
      b.requests - a.requests ||
      a.modelName.localeCompare(b.modelName)
  )
  const maxWeight = Math.max(0, ...models.map(weightOf))
  const totalWeight = models.reduce((sum, model) => sum + weightOf(model), 0)
  const ranked = models.map((model) => ({
    ...model,
    intensity: maxWeight > 0 ? weightOf(model) / maxWeight : 0,
    share: totalWeight > 0 ? weightOf(model) / totalWeight : 0,
  }))

  return {
    models: ranked,
    shareMetric: weightMetric,
    totals: {
      requests: totalRequests,
      tokens: totalTokens,
      quota: totalQuota,
      outputImages: models.reduce((sum, model) => sum + model.outputImages, 0),
      sponsoredRequests: models.reduce(
        (sum, model) => sum + model.sponsoredRequests,
        0
      ),
      sponsoredQuota: sumUsageCredits(
        models.map((model) => model.sponsoredQuota)
      ),
      tokenCountScope: models.some(
        (model) => model.tokenCountScope === 'settled_text_requests'
      )
        ? 'settled_text_requests'
        : null,
      modelCount: ranked.length,
      topModel: ranked[0] ?? null,
    },
  }
}

/** Resolve the timestamps for a report range, clamped to account age. */
export function getModelUsageRange(
  rangeKey: ModelUsageRangeKey,
  now = new Date(),
  accountCreatedTime?: number
): ProfileUsageQueryRange {
  const endTimestamp = Math.floor(now.getTime() / 1000)
  const dayCount =
    rangeKey === '7d' ? 7 : rangeKey === '30d' ? 30 : MODEL_USAGE_YEAR_DAYS
  const startTimestamp = endTimestamp - dayCount * 24 * 60 * 60
  const created = Number(accountCreatedTime)

  return {
    start_timestamp:
      Number.isFinite(created) && created > 0 && created > startTimestamp
        ? Math.min(endTimestamp, Math.floor(created))
        : startTimestamp,
    end_timestamp: endTimestamp,
  }
}

export function buildModelUsageQueryRanges(
  range: ProfileUsageQueryRange
): ProfileUsageQueryRange[] {
  return buildProfileUsageQueryRanges(
    range.start_timestamp,
    range.end_timestamp,
    MODEL_USAGE_WINDOW_SECONDS
  )
}
