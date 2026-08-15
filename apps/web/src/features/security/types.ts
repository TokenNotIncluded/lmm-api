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
*/

export type SecurityRiskCategory = {
  id: string
  name: string
  layer: string
  severity: string
  description: string
  source: string
}

export type SecurityRuleSummary = {
  id: string
  name: string
  category: string
  layer: string
  severity: string
  source: string
  version: string
  description: string
}

export type SecurityViolationFeeRule = {
  code: string
  provider: string
  trigger: string
  enabled: boolean
  amount_usd: number
  charge_unit: string
  retryable: boolean
  description: string
  charging_notes: string
  local_guardrail_fee: boolean
}

export type SecurityPolicy = {
  policy_version: string
  reference_effective_date: string
  reference_url: string
  alignment: string
  enforcement: {
    enabled: boolean
    on_prompt: boolean
    action: 'block' | 'audit'
  }
  protected_groups?: string[]
  risk_categories: SecurityRiskCategory[]
  rules: SecurityRuleSummary[]
  violation_fees: SecurityViolationFeeRule[]
}

export type SecurityStatsBucket = {
  key: string
  count: number
}

export type SecurityStats = {
  start_timestamp: number
  end_timestamp: number
  total_matches: number
  blocked_matches: number
  audited_matches: number
  affected_requests: number
  affected_users: number
  by_category: SecurityStatsBucket[]
  by_rule?: SecurityStatsBucket[]
}

export type SecurityPolicyResponse = {
  success: boolean
  message?: string
  data?: SecurityPolicy
}

export type SecurityStatsResponse = {
  success: boolean
  message?: string
  data?: SecurityStats
}
