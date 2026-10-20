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
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { adaptNativePricing } from './catalog-adapter'
import { nativePricingSchema } from './catalog-contract'
import type { PricingData } from './types'

// ----------------------------------------------------------------------------
// Pricing APIs
// ----------------------------------------------------------------------------

// Get model pricing data
export async function getPricing(signal?: AbortSignal): Promise<PricingData> {
  const { user, session } = useAuthStore.getState().auth
  const visibility = user ? 'session' : 'public'
  const res = await api.get<unknown>(`/core/v1/pricing/${visibility}`, {
    signal,
    timeout: 10_000,
    authScope: { userId: user?.id, sessionId: session?.sid },
    authMode: user ? 'session' : 'anonymous',
    skipAuthRefresh: !user,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  const document = nativePricingSchema.parse(res.data)
  if (document.visibility !== visibility) {
    throw new Error('Model catalog visibility did not match the request')
  }
  return adaptNativePricing(document)
}

export function pricingQueryKey(userId?: number, sessionId?: string) {
  return ['pricing', userId ?? null, sessionId ?? null]
}
