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
import { useQuery } from '@tanstack/react-query'
import { useMemo } from 'react'

import { useStatus } from '@/hooks/use-status'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { useAuthStore } from '@/stores/auth-store'

import { getPricing, pricingQueryKey } from '../api'
import { useModelRuntime } from './use-model-runtime'

export function usePricingData(options?: { enabled?: boolean }) {
  useStatus()
  const { user, session } = useAuthStore((state) => state.auth)
  const {
    currency: displayCurrency,
    setPreference: setDisplayCurrency,
    error: displayCurrencyError,
  } = useWalletCurrency()

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: pricingQueryKey(user?.id, session?.sid),
    queryFn: ({ signal }) => getPricing(signal),
    enabled: options?.enabled ?? true,
    staleTime: 5 * 60 * 1000,
    retry: false,
  })

  const models = useMemo(() => {
    if (!data?.data || !data?.vendors) return []

    const vendorMap = new Map(data.vendors.map((v) => [v.id, v]))

    return data.data.map((model) => {
      const vendor = model.vendor_id
        ? vendorMap.get(model.vendor_id)
        : undefined
      return {
        ...model,
        key: model.model_name,
        vendor_name: vendor?.name,
        vendor_icon: vendor?.icon,
        vendor_description: vendor?.description,
        group_ratio: data.group_ratio,
      }
    })
  }, [data])

  const runtime = useModelRuntime(
    models.map((model) => model.model_name),
    (options?.enabled ?? true) && data?.runtime_available === true
  )
  const liveModels = useMemo(
    () =>
      models.map((model) => ({
        ...model,
        runtime_state: runtime[model.model_name] ?? model.runtime_state,
      })),
    [models, runtime]
  )
  return {
    models: liveModels,
    vendors: data?.vendors ?? [],
    groupRatio: data?.group_ratio ?? {},
    usableGroup: data?.usable_group ?? {},
    endpointMap: data?.supported_endpoint ?? {},
    autoGroups: data?.auto_groups ?? [],
    performanceAvailable: data?.performance_available === true,
    runtimeAvailable: data?.runtime_available === true,
    isLoading,
    error,
    refetch,
    displayCurrency,
    setDisplayCurrency,
    displayCurrencyError,
  }
}
