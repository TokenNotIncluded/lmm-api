/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type {
  ColumnDef,
  ColumnFiltersState,
  OnChangeFn,
  PaginationState,
} from '@tanstack/react-table'
import { useTranslation } from 'react-i18next'

import { DataTablePage, useDataTable } from '@/components/data-table'
import { useMediaQuery } from '@/hooks'
import { useHistoricalQuotaMoney } from '@/hooks/use-historical-quota-money'
import { formatTimestampToDate } from '@/lib/format'

import type { HistoricalLog } from '../lib/history-response'
import { CommonLogsFilterBar } from './common-logs-filter-bar'
import { useUsageLogsContext } from './usage-logs-provider'

interface HistoricalLogsTableProps {
  items: HistoricalLog[]
  total: number
  pagination: PaginationState
  onPaginationChange: OnChangeFn<PaginationState>
  columnFilters: ColumnFiltersState
  onColumnFiltersChange: OnChangeFn<ColumnFiltersState>
  ensurePageInRange: (pageCount: number) => void
  isFetching: boolean
}

/** Historical rows carry their original ID and nullable facts, not native receipts. */
export function HistoricalLogsTable(props: HistoricalLogsTableProps) {
  const { t } = useTranslation()
  const isMobile = useMediaQuery('(max-width: 640px)')
  const { sensitiveVisible } = useUsageLogsContext()
  const { formatHistoricalQuota } = useHistoricalQuotaMoney()
  const columns: ColumnDef<HistoricalLog>[] = [
    {
      accessorKey: 'created_at',
      header: t('Time'),
      cell: ({ row }) => formatTimestampToDate(row.original.created_at),
    },
    {
      accessorKey: 'source_log_id',
      header: t('ID'),
      cell: ({ row }) => String(row.original.source_log_id),
    },
    {
      accessorKey: 'username',
      header: t('Username'),
      cell: ({ row }) =>
        sensitiveVisible ? row.original.username || '—' : '••••',
    },
    {
      accessorKey: 'model_name',
      header: t('Model'),
      cell: ({ row }) => row.original.model_name || '—',
    },
    {
      accessorKey: 'quota',
      header: t('Usage'),
      cell: ({ row }) =>
        row.original.quota === null
          ? '—'
          : formatHistoricalQuota(row.original.quota, row.original),
    },
    {
      accessorKey: 'prompt_tokens',
      header: t('Input Tokens'),
      cell: ({ row }) => row.original.prompt_tokens?.toLocaleString() ?? '—',
    },
    {
      accessorKey: 'completion_tokens',
      header: t('Output Tokens'),
      cell: ({ row }) =>
        row.original.completion_tokens?.toLocaleString() ?? '—',
    },
    {
      accessorKey: 'token_name',
      header: t('Token Name'),
      cell: ({ row }) =>
        sensitiveVisible ? row.original.token_name || '—' : '••••',
    },
    {
      accessorKey: 'group',
      header: t('Group'),
      cell: ({ row }) => row.original.group || '—',
    },
    {
      accessorKey: 'request_id',
      header: t('Request ID'),
      cell: ({ row }) => row.original.request_id || '—',
    },
  ]
  const { table } = useDataTable({
    data: props.items,
    columns,
    pagination: props.pagination,
    onPaginationChange: props.onPaginationChange,
    columnFilters: props.columnFilters,
    onColumnFiltersChange: props.onColumnFiltersChange,
    totalCount: props.total,
    ensurePageInRange: props.ensurePageInRange,
    enableRowSelection: false,
    manualFiltering: true,
    manualPagination: true,
    enableSorting: false,
    columnVisibilityStorageKey: 'usage-logs:historical:column-visibility',
  })
  return (
    <DataTablePage
      table={table}
      columns={columns}
      isFetching={props.isFetching}
      fixedHeight={!isMobile}
      paginationInFooter={!isMobile}
      emptyTitle={t('No Logs Found')}
      toolbar={
        <div className='space-y-3'>
          <div role='note' className='text-muted-foreground text-sm'>
            <p className='text-foreground font-medium'>
              {t('Imported history')}
            </p>
            <p>
              {t(
                'This view shows imported historical logs. New requests for your account appear under Only Mine.'
              )}
            </p>
          </div>
          <CommonLogsFilterBar table={table} />
        </div>
      }
    />
  )
}
