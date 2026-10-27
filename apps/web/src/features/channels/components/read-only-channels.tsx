/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { getChannels, searchChannels } from '../api'

const PAGE_SIZE = 10

/** The routing snapshot supplies configuration facts, not legacy operational metrics. */
export function ReadOnlyChannels() {
  const { t } = useTranslation()
  const [input, setInput] = useState('')
  const [keyword, setKeyword] = useState('')
  const [page, setPage] = useState(1)
  // @context-fold begin read-only-channel-query
  const query = useQuery({
    queryKey: ['channels', 'read-only', keyword, page],
    queryFn: () =>
      keyword
        ? searchChannels({ keyword, p: page, page_size: PAGE_SIZE })
        : getChannels({ p: page, page_size: PAGE_SIZE }),
    retry: false,
  })
  const data = query.data?.data
  // @context-fold end read-only-channel-query

  return (
    <div className='min-w-0 space-y-4'>
      <Alert>
        <AlertTitle>{t('Read only')}</AlertTitle>
        <AlertDescription>
          {t(
            'Channels are read-only. Names and status come from the active routing configuration; balances and test results are unavailable.'
          )}
        </AlertDescription>
      </Alert>
      <form
        className='flex flex-col gap-2 sm:flex-row'
        onSubmit={(event) => {
          event.preventDefault()
          setKeyword(input.trim())
          setPage(1)
        }}
      >
        <Input
          className='min-h-11 min-w-0'
          value={input}
          onChange={(event) => setInput(event.target.value)}
          aria-label={t('Search')}
        />
        <Button type='submit' className='min-h-11 shrink-0'>
          {t('Search')}
        </Button>
      </form>
      {query.isLoading ? (
        <LoadingState />
      ) : query.isError ? (
        <ErrorState
          description={
            getServerErrorMessage(query.error) ?? t('Failed to load data')
          }
          onRetry={() => void query.refetch()}
        />
      ) : data ? (
        <>
          <p className='text-muted-foreground text-sm'>
            {t('Total')}: {data.total}
          </p>
          <div className='space-y-3 sm:hidden'>
            {data.items.map((channel) => (
              <dl key={channel.id} className='min-w-0 space-y-3 border p-3'>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>{t('Name')}</dt>
                  <dd className='font-medium [overflow-wrap:anywhere]'>
                    {channel.name}
                  </dd>
                </div>
                <div className='grid min-w-0 grid-cols-2 gap-3'>
                  <div className='min-w-0'>
                    <dt className='text-muted-foreground text-xs'>{t('ID')}</dt>
                    <dd className='font-mono text-sm [overflow-wrap:anywhere]'>
                      {channel.id}
                    </dd>
                  </div>
                  <div className='min-w-0'>
                    <dt className='text-muted-foreground text-xs'>
                      {t('Status')}
                    </dt>
                    <dd className='text-sm [overflow-wrap:anywhere]'>
                      {channel.status === 1 ? t('Enabled') : t('Disabled')}
                    </dd>
                  </div>
                </div>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>
                    {t('Group')}
                  </dt>
                  <dd className='text-sm [overflow-wrap:anywhere] whitespace-pre-wrap'>
                    {channel.group}
                  </dd>
                </div>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>
                    {t('Models')}
                  </dt>
                  <dd className='text-sm [overflow-wrap:anywhere] whitespace-pre-wrap'>
                    {channel.models}
                  </dd>
                </div>
              </dl>
            ))}
          </div>
          <div className='hidden overflow-x-auto border sm:block'>
            <table className='w-full text-left text-sm'>
              <thead className='border-b'>
                <tr>
                  {['ID', 'Name', 'Status', 'Group', 'Models'].map((label) => (
                    <th key={label} className='p-3'>
                      {t(label)}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className='divide-y'>
                {data.items.map((channel) => (
                  <tr key={channel.id}>
                    <td className='p-3'>{channel.id}</td>
                    <td className='p-3'>{channel.name}</td>
                    <td className='p-3'>
                      {channel.status === 1 ? t('Enabled') : t('Disabled')}
                    </td>
                    <td className='p-3'>{channel.group}</td>
                    <td className='max-w-md p-3 break-words whitespace-pre-wrap'>
                      {channel.models}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {data.items.length === 0 ? (
            <p className='text-muted-foreground border p-3'>
              {t('No data available')}
            </p>
          ) : null}
          <div className='flex flex-wrap items-center justify-end gap-3'>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={page === 1}
              onClick={() => setPage(page - 1)}
            >
              {t('Previous')}
            </Button>
            <span className='text-sm'>
              {t('Page')} {page} /{' '}
              {Math.max(1, Math.ceil(data.total / PAGE_SIZE))}
            </span>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={page * PAGE_SIZE >= data.total}
              onClick={() => setPage(page + 1)}
            >
              {t('Next')}
            </Button>
          </div>
        </>
      ) : null}
    </div>
  )
}
