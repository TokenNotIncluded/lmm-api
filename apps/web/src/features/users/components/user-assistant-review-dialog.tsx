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
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ChevronLeft, ChevronRight, RotateCcw, ShieldAlert } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { formatTimestamp } from '@/lib/format'
import { useAuthStore } from '@/stores/auth-store'

import {
  listAssistantRequestReviews,
  resetAssistantRequestReviewViolations,
} from '../api'
import { canViewUserAssistantHistory } from '../lib/assistant-history-access'
import type { User } from '../types'

const REVIEW_PAGE_SIZE = 20

export function UserAssistantReviewDialog(props: { user: User }) {
  const { t } = useTranslation()
  const viewer = useAuthStore((state) => state.auth.user)
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [page, setPage] = useState(1)
  const count = props.user.assistant_violation_count ?? 0
  const canView = canViewUserAssistantHistory(viewer, props.user)
  const reviewsQuery = useQuery({
    queryKey: ['assistant-request-reviews', props.user.id, page],
    queryFn: async () => {
      const response = await listAssistantRequestReviews(
        props.user.id,
        page,
        REVIEW_PAGE_SIZE
      )
      if (!response.success) {
        throw new Error(response.message || t('Unable to load review logs'))
      }
      return response.data
    },
    enabled: open,
  })
  const reviewData = reviewsQuery.data
  const total = reviewData?.total ?? 0
  const pageSize = Math.max(1, reviewData?.page_size ?? REVIEW_PAGE_SIZE)
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const resetMutation = useMutation({
    mutationFn: () => resetAssistantRequestReviewViolations(props.user.id),
    onSuccess: async (response) => {
      if (!response.success) {
        toast.error(response.message || t('Unable to reset violations'))
        return
      }
      toast.success(t('Violation count reset'))
      await queryClient.invalidateQueries({
        queryKey: ['assistant-request-reviews', props.user.id],
      })
      await queryClient.invalidateQueries({ queryKey: ['users'] })
    },
    onError: (error: Error) => toast.error(error.message),
  })

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen)
    if (nextOpen) {
      setPage(1)
    }
  }

  if (!canView) {
    return <span className='text-muted-foreground'>—</span>
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Button
        type='button'
        variant={count > 0 ? 'destructive' : 'outline'}
        size='sm'
        className='min-h-11 sm:min-h-7'
        onClick={() => handleOpenChange(true)}
        aria-label={`${t('Violations')}: ${count}`}
      >
        <ShieldAlert aria-hidden='true' />
        {count.toLocaleString()}
      </Button>
      <DialogContent className='flex max-h-[min(52rem,calc(100svh-2rem))] min-h-0 w-[calc(100%-1rem)] flex-col sm:max-w-3xl'>
        <DialogHeader>
          <DialogTitle>{t('Assistant review logs')}</DialogTitle>
          <DialogDescription>
            {props.user.username} · {t('User ID')}: {props.user.id} ·{' '}
            {t('Current violations')}: {count.toLocaleString()}
          </DialogDescription>
        </DialogHeader>
        <div className='min-h-0 flex-1 space-y-3 overflow-y-auto pr-1'>
          {reviewsQuery.isLoading && (
            <p className='text-muted-foreground text-sm'>{t('Loading')}</p>
          )}
          {reviewsQuery.isError && (
            <p className='text-destructive text-sm'>
              {reviewsQuery.error.message}
            </p>
          )}
          {!reviewsQuery.isLoading &&
            !reviewsQuery.isError &&
            (reviewsQuery.data?.items.length ?? 0) === 0 && (
              <p className='text-muted-foreground text-sm'>
                {t('No sampled reviews')}
              </p>
            )}
          {reviewsQuery.data?.items.map((review) => (
            <article
              key={review.id}
              className='border-border/70 space-y-2 border-b pb-3 last:border-b-0'
            >
              <div className='flex flex-wrap items-center justify-between gap-2 text-sm'>
                <span className='font-medium'>
                  {review.violation ? t('Violation') : t('No violation')}
                  {review.abuse ? ` · ${t('Possible abuse')}` : ''}
                </span>
                <span className='text-muted-foreground'>
                  {formatTimestamp(review.created_at)}
                </span>
              </div>
              <div className='text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs'>
                <span>{review.group || t('Default group')}</span>
                <span>{review.review_model}</span>
                <span>{review.intensity}</span>
                <span>{review.status}</span>
              </div>
              {review.rules.length > 0 && (
                <p className='text-sm'>
                  <span className='font-medium'>{t('Rules')}: </span>
                  {review.rules.join(', ')}
                </p>
              )}
              {review.explanation && (
                <p className='text-sm'>
                  <span className='font-medium'>{t('Explanation')}: </span>
                  {review.explanation}
                </p>
              )}
              {review.request_preview && (
                <p className='text-muted-foreground line-clamp-3 text-xs'>
                  <span className='font-medium'>{t('Request preview')}: </span>
                  {review.request_preview}
                </p>
              )}
            </article>
          ))}
        </div>
        {reviewData && total > 0 && (
          <div className='text-muted-foreground flex flex-wrap items-center justify-between gap-3 border-t pt-3 text-xs tabular-nums'>
            <div className='flex flex-wrap gap-x-3 gap-y-1'>
              <span>
                {t('Total')}: {total.toLocaleString()}
              </span>
              {totalPages > 1 && (
                <span>
                  {t('Page {{page}} of {{total}}', {
                    page,
                    total: totalPages,
                  })}
                </span>
              )}
            </div>
            {totalPages > 1 && (
              <div className='flex items-center gap-1'>
                <Button
                  type='button'
                  variant='outline'
                  size='icon-sm'
                  aria-label={t('Previous page')}
                  onClick={() => setPage((current) => Math.max(1, current - 1))}
                  disabled={page <= 1 || reviewsQuery.isFetching}
                >
                  <ChevronLeft aria-hidden='true' />
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  size='icon-sm'
                  aria-label={t('Next page')}
                  onClick={() =>
                    setPage((current) => Math.min(totalPages, current + 1))
                  }
                  disabled={page >= totalPages || reviewsQuery.isFetching}
                >
                  <ChevronRight aria-hidden='true' />
                </Button>
              </div>
            )}
          </div>
        )}
        <DialogFooter>
          <Button
            type='button'
            variant='outline'
            onClick={() => resetMutation.mutate()}
            disabled={resetMutation.isPending || count === 0}
          >
            <RotateCcw aria-hidden='true' />
            {resetMutation.isPending ? t('Resetting...') : t('Reset count')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
