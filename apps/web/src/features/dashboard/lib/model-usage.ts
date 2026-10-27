/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import type { QuotaDataItem } from '../types'

export type ModelUsageMetric = 'count' | 'tokens' | 'quota'
export type ModelUsageSort = {
  metric: ModelUsageMetric
  direction: 'ascending' | 'descending'
}

export interface ModelUsageRow {
  model: string
  count: number
  tokens: number | null
  quota: number
  quota_raw: string | null
  share: number | null
  output_image_count?: number | null
  token_count_scope?: QuotaDataItem['token_count_scope']
  usage_scope?: QuotaDataItem['usage_scope']
  settlement_scope?: QuotaDataItem['settlement_scope']
  platform_sponsored_quota?: number | null
  wallet_debit_supported?: false
}

const finite = (value: number | null | undefined) =>
  typeof value === 'number' && Number.isFinite(value) ? value : 0

function knownTotal(
  values: readonly (number | null | undefined)[]
): number | null {
  let total = 0
  for (const value of values) {
    if (
      typeof value !== 'number' ||
      !Number.isSafeInteger(value) ||
      value < 0
    ) {
      return null
    }
    total += value
    if (!Number.isSafeInteger(total)) return null
  }
  return total
}

function usageMetadata(inputs: readonly QuotaDataItem[]) {
  // @context-fold begin aggregate-usage-metadata
  return {
    ...(inputs.some((item) => item.output_image_count !== undefined)
      ? {
          output_image_count: knownTotal(
            inputs.map((item) => item.output_image_count)
          ),
        }
      : {}),
    ...(inputs.every(
      (item) => item.token_count_scope === 'settled_text_requests'
    )
      ? { token_count_scope: 'settled_text_requests' as const }
      : {}),
    ...(inputs.every((item) => item.usage_scope !== undefined)
      ? {
          usage_scope: inputs.some(
            (item) => item.usage_scope === 'administrator_all_accounts'
          )
            ? ('administrator_all_accounts' as const)
            : inputs.some((item) => item.usage_scope === 'actor_personal_usage')
              ? ('actor_personal_usage' as const)
              : ('actor_personal_key' as const),
        }
      : {}),
    ...(inputs.every((item) => item.settlement_scope === 'settled_requests')
      ? { settlement_scope: 'settled_requests' as const }
      : {}),
    ...(inputs.some((item) => item.platform_sponsored !== undefined)
      ? {
          platform_sponsored_quota: inputs.every(
            (item) => item.platform_sponsored !== undefined
          )
            ? knownTotal(
                inputs.map((item) => (item.platform_sponsored ? item.quota : 0))
              )
            : null,
        }
      : {}),
    ...(inputs.every((item) => item.wallet_debit_supported === false)
      ? { wallet_debit_supported: false as const }
      : {}),
  }
  // @context-fold end aggregate-usage-metadata
}

/** Keep model identity intact; translate an empty name only when displaying it. */
export function summarizeModelUsage(data: readonly QuotaDataItem[]) {
  const models = new Map<string, ModelUsageRow>()
  const modelInputs = new Map<string, QuotaDataItem[]>()
  const unavailableTokens = new Set<string>()
  let totalQuota = 0n
  let completeRawQuota = true
  for (const item of data) {
    const model = item.model_name ?? ''
    const inputs = modelInputs.get(model) ?? []
    inputs.push(item)
    modelInputs.set(model, inputs)
    let row = models.get(model)
    if (!row) {
      row = {
        model,
        count: 0,
        tokens: null,
        quota: 0,
        quota_raw: '0',
        share: null,
      }
      models.set(model, row)
    }
    row.count += finite(item.count)
    // Native image-only buckets have no token receipt. A mixed model's known
    // text sum remains valid and retains its settled-text-only scope below.
    const imageOnly =
      item.token_used === null &&
      item.token_count_scope === 'settled_text_requests' &&
      typeof item.output_image_count === 'number' &&
      item.output_image_count > 0
    if (
      typeof item.token_used === 'number' &&
      Number.isFinite(item.token_used)
    ) {
      row.tokens = (row.tokens ?? 0) + item.token_used
    } else if (!imageOnly) {
      unavailableTokens.add(model)
    }
    row.quota += finite(item.quota)
    const raw =
      item.quota_money_basis !== 'invalid_stored_quota' &&
      typeof item.quota === 'number' &&
      Number.isSafeInteger(item.quota)
        ? BigInt(item.quota)
        : null
    row.quota_raw =
      row.quota_raw !== null && raw !== null
        ? (BigInt(row.quota_raw) + raw).toString()
        : null
    if (raw === null) {
      completeRawQuota = false
    } else {
      totalQuota += raw
    }
  }
  return Array.from(models.values(), (row) => ({
    ...row,
    tokens: unavailableTokens.has(row.model) ? null : row.tokens,
    ...usageMetadata(modelInputs.get(row.model) ?? []),
    // Unknown input makes the entire period's denominator unknown. A known
    // row must not acquire 100% by silently treating another row as zero.
    share:
      completeRawQuota && totalQuota > 0n && row.quota_raw !== null
        ? Number(BigInt(row.quota_raw)) / Number(totalQuota)
        : null,
  }))
}

export function selectModelUsage(
  rows: readonly ModelUsageRow[],
  query: string,
  sort: ModelUsageSort,
  unknownLabel: string
) {
  const needle = query.trim().toLocaleLowerCase()
  const direction = sort.direction === 'ascending' ? 1 : -1
  return rows
    .filter((row) =>
      (row.model || unknownLabel).toLocaleLowerCase().includes(needle)
    )
    .sort((a, b) => {
      const left = a[sort.metric]
      const right = b[sort.metric]
      // Unknown token counts sort after known values in either direction.
      if (left === null && right !== null) return 1
      if (right === null && left !== null) return -1
      const difference =
        left === null || right === null ? 0 : (left - right) * direction
      // Stable tie order regardless of API row order, without mutating input.
      return difference || (a.model < b.model ? -1 : a.model > b.model ? 1 : 0)
    })
}

function csvCell(value: string | number | bigint) {
  let text = String(value)
  // Quoting alone does not stop a spreadsheet from executing a model name.
  if (typeof value === 'string' && /^[\s\uFEFF]*[=+@-]/u.test(text)) {
    text = `'${text}`
  }
  return `"${text.replaceAll('"', '""')}"`
}

export function modelUsageCsv(
  rows: readonly ModelUsageRow[],
  headers: readonly string[],
  unknownLabel: string
) {
  const nativeDetails = rows.some(
    (row) =>
      row.output_image_count !== undefined ||
      row.token_count_scope !== undefined
  )
  const records: (string | number | bigint)[][] = [
    [...headers],
    ...rows.map((row) => [
      row.model || unknownLabel,
      row.count,
      row.tokens ?? '',
      row.quota_raw === null ? '' : BigInt(row.quota_raw),
      // Hour buckets do not capture a historical money basis. Aggregating raw
      // quota cannot establish one, even if today's wallet FX is available.
      'raw_quota',
      'false',
      row.quota_raw === null
        ? 'raw_quota_not_exact'
        : 'bucket_without_credit_basis',
      row.share === null ? '' : row.share * 100,
      ...(nativeDetails
        ? [row.output_image_count ?? '', row.token_count_scope ?? '']
        : []),
    ]),
  ]
  // BOM preserves non-ASCII model names in common spreadsheet applications.
  return `\uFEFF${records.map((record) => record.map(csvCell).join(',')).join('\r\n')}\r\n`
}
