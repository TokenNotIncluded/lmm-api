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
import { buildModelUsageReport } from '@/features/profile/lib/model-usage'

export function summarizeAssistantUsage(rows: QuotaDataItem[]) {
  const report = buildModelUsageReport(rows)
  return {
    requests: report.totals.requests,
    tokens: report.totals.tokens,
    credits: report.totals.quota,
    outputImages: report.totals.outputImages,
    sponsoredRequests: report.totals.sponsoredRequests,
    sponsoredCredits: report.totals.sponsoredQuota,
    tokenCountScope: report.totals.tokenCountScope,
    models: report.models
      .map((model) => ({
        model: model.modelName === 'unknown' ? '' : model.modelName,
        requests: model.requests,
        tokens: model.tokens,
        credits: model.quota,
        outputImages: model.outputImages,
        sponsoredRequests: model.sponsoredRequests,
        sponsoredCredits: model.sponsoredQuota,
        tokenCountScope: model.tokenCountScope,
        sharePercent: model.share * 100,
      }))
      .filter(
        (model) =>
          model.requests > 0 ||
          (model.tokens ?? 0) > 0 ||
          (model.credits ?? 0) > 0 ||
          model.outputImages > 0
      ),
  }
}

export type AssistantUsageSummary = ReturnType<typeof summarizeAssistantUsage>
export type AssistantUsageModelSummary = AssistantUsageSummary['models'][number]
