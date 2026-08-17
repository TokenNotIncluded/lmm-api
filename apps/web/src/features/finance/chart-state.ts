/*
Copyright (C) 2026 LIghtJUNction
*/

export type FinanceChartState = 'error' | 'loading' | 'empty' | 'ready'

export function financeChartState(options: {
  hasError: boolean
  isLoading: boolean
  pointCount: number
}): FinanceChartState {
  if (options.hasError) return 'error'
  if (options.isLoading) return 'loading'
  return options.pointCount > 0 ? 'ready' : 'empty'
}
