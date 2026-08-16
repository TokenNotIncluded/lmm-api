/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Switch } from '@/components/ui/switch'

import {
  createDiscountCode,
  deleteDiscountCode,
  listDiscountCodes,
  updateDiscountCode,
  updateDiscountCodeStatus,
} from './api'
import {
  DISCOUNT_CODE_ENABLED_STATUS,
  getDiscountCodeAvailability,
  parseDiscountCodeMaxUses,
} from './availability'
import type { DiscountCode, DiscountCodeInput } from './types'

const DISABLED = 2

type FormState = {
  code: string
  name: string
  discount_percent: string
  min_amount: string
  max_uses: string
  starts_time: string
  expired_time: string
}

const emptyForm: FormState = {
  code: '',
  name: '',
  discount_percent: '10',
  min_amount: '0',
  max_uses: '0',
  starts_time: '',
  expired_time: '',
}

function toDateInput(timestamp: number) {
  if (!timestamp) return ''
  const date = new Date(timestamp * 1000)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function toTimestamp(value: string) {
  if (!value) return 0
  const timestamp = Math.floor(new Date(value).getTime() / 1000)
  return Number.isFinite(timestamp) ? timestamp : 0
}

function formFromRow(row?: DiscountCode): FormState {
  if (!row) return emptyForm
  return {
    code: row.code,
    name: row.name,
    discount_percent: String(row.discount_percent),
    min_amount: String(row.min_amount),
    max_uses: String(row.max_uses),
    starts_time: toDateInput(row.starts_time),
    expired_time: toDateInput(row.expired_time),
  }
}

function formatDate(timestamp: number) {
  if (!timestamp) return '—'
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(timestamp * 1000))
}

function availabilityLabel(
  availability: ReturnType<typeof getDiscountCodeAvailability>,
  t: ReturnType<typeof useTranslation>['t']
) {
  switch (availability) {
    case 'active':
      return t('Active')
    case 'not_started':
      return t('Not Started')
    case 'expired':
      return t('Expired')
    default:
      return t('Disabled')
  }
}

export function DiscountCodes() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [keyword, setKeyword] = useState('')
  const [page, setPage] = useState(1)
  const [sheetOpen, setSheetOpen] = useState(false)
  const [editing, setEditing] = useState<DiscountCode>()
  const [form, setForm] = useState<FormState>(emptyForm)

  const query = useQuery({
    queryKey: ['discount-codes', page, keyword],
    queryFn: () => listDiscountCodes({ page, pageSize: 20, keyword }),
    placeholderData: (previous) => previous,
  })
  const rows = query.data?.data?.items ?? []
  const total = query.data?.data?.total ?? 0
  const pageCount = Math.max(1, Math.ceil(total / 20))

  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ['discount-codes'] })

  const saveMutation = useMutation({
    mutationFn: async (input: DiscountCodeInput) =>
      editing
        ? updateDiscountCode({ ...input, id: editing.id })
        : createDiscountCode(input),
    onSuccess: (result) => {
      if (!result.success) {
        toast.error(result.message || t('Unable to save discount code'))
        return
      }
      toast.success(t('Discount code saved'))
      setSheetOpen(false)
      refresh()
    },
    onError: (error) =>
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to save discount code')
      ),
  })

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: number; status: number }) =>
      updateDiscountCodeStatus(id, status),
    onSuccess: (result) => {
      if (!result.success) {
        toast.error(result.message || t('Unable to update discount code'))
        return
      }
      refresh()
    },
  })

  const deleteMutation = useMutation({
    mutationFn: deleteDiscountCode,
    onSuccess: (result) => {
      if (!result.success) {
        toast.error(result.message || t('Unable to delete discount code'))
        return
      }
      toast.success(t('Discount code deleted'))
      refresh()
    },
  })

  const isSaving = saveMutation.isPending
  const maxUses = parseDiscountCodeMaxUses(form.max_uses)
  const canSave = useMemo(
    () =>
      form.code.trim().length >= 3 &&
      form.name.trim().length > 0 &&
      Number(form.discount_percent) >= 1 &&
      Number(form.discount_percent) <= 99 &&
      Number(form.min_amount) >= 0 &&
      maxUses !== undefined,
    [form, maxUses]
  )

  const openCreate = () => {
    setEditing(undefined)
    setForm(emptyForm)
    setSheetOpen(true)
  }

  const openEdit = (row: DiscountCode) => {
    setEditing(row)
    setForm(formFromRow(row))
    setSheetOpen(true)
  }

  const submit = () => {
    if (!canSave || maxUses === undefined) return
    saveMutation.mutate({
      code: form.code.trim().toUpperCase(),
      name: form.name.trim(),
      discount_percent: Number(form.discount_percent),
      min_amount: Math.max(0, Math.floor(Number(form.min_amount))),
      max_uses: maxUses,
      starts_time: toTimestamp(form.starts_time),
      expired_time: toTimestamp(form.expired_time),
    })
  }

  return (
    <>
      <SectionPageLayout>
        <SectionPageLayout.Title>{t('Discount Codes')}</SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <div className='flex gap-2'>
            <Button
              variant='outline'
              size='sm'
              onClick={() => void query.refetch()}
            >
              <RefreshCw className='size-4' />
              {t('Refresh')}
            </Button>
            <Button size='sm' onClick={openCreate}>
              <Plus className='size-4' />
              {t('Create discount code')}
            </Button>
          </div>
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='mx-auto w-full max-w-6xl space-y-5'>
            <p className='text-muted-foreground text-sm'>
              {t(
                'Manage percentage discounts for checkout. Codes are validated and applied by the server.'
              )}
            </p>
            <Input
              value={keyword}
              onChange={(event) => {
                setKeyword(event.target.value)
                setPage(1)
              }}
              placeholder={t('Filter by code or name...')}
              className='max-w-sm'
            />
            <div className='overflow-x-auto'>
              <div className='min-w-[900px]'>
                <div className='text-muted-foreground grid grid-cols-[1.3fr_1fr_.7fr_.8fr_1fr_1.4fr_auto] gap-4 border-b px-2 pb-3 text-xs tracking-wider uppercase'>
                  <span>{t('Code')}</span>
                  <span>{t('Name')}</span>
                  <span>{t('Discount')}</span>
                  <span>{t('Used')}</span>
                  <span>{t('Status')}</span>
                  <span>{t('Validity')}</span>
                  <span />
                </div>
                {query.isLoading ? (
                  <p className='text-muted-foreground px-2 py-10 text-sm'>
                    {t('Loading...')}
                  </p>
                ) : rows.length === 0 ? (
                  <p className='text-muted-foreground px-2 py-10 text-sm'>
                    {t('No discount codes')}
                  </p>
                ) : (
                  rows.map((row) => (
                    <div
                      className='grid grid-cols-[1.3fr_1fr_.7fr_.8fr_1fr_1.4fr_auto] items-center gap-4 border-b px-2 py-4 text-sm last:border-b-0'
                      key={row.id}
                    >
                      <div className='font-mono font-medium'>{row.code}</div>
                      <div className='truncate'>{row.name}</div>
                      <div>{row.discount_percent}%</div>
                      <div className='tabular-nums'>
                        {row.used_count} /{' '}
                        {row.max_uses > 0 ? row.max_uses : t('No maximum')}
                      </div>
                      <div className='flex items-center gap-2'>
                        <Switch
                          size='sm'
                          checked={row.status === DISCOUNT_CODE_ENABLED_STATUS}
                          aria-label={`${row.code} ${t('Enabled')}`}
                          onCheckedChange={(checked) =>
                            statusMutation.mutate({
                              id: row.id,
                              status: checked
                                ? DISCOUNT_CODE_ENABLED_STATUS
                                : DISABLED,
                            })
                          }
                        />
                        <span className='text-muted-foreground text-xs'>
                          {availabilityLabel(
                            getDiscountCodeAvailability(row),
                            t
                          )}
                        </span>
                      </div>
                      <div className='text-muted-foreground space-y-0.5 text-xs'>
                        <p>
                          {t('Starts')}: {formatDate(row.starts_time)}
                        </p>
                        <p>
                          {t('Expires')}:{' '}
                          {row.expired_time
                            ? formatDate(row.expired_time)
                            : t('Never expires')}
                        </p>
                      </div>
                      <div className='flex justify-end gap-1'>
                        <Button
                          variant='ghost'
                          size='icon'
                          onClick={() => openEdit(row)}
                          aria-label={t('Edit')}
                        >
                          <Pencil className='size-4' />
                        </Button>
                        <Button
                          variant='ghost'
                          size='icon'
                          className='text-destructive'
                          onClick={() => {
                            if (
                              window.confirm(t('Delete this discount code?'))
                            ) {
                              deleteMutation.mutate(row.id)
                            }
                          }}
                          aria-label={t('Delete')}
                        >
                          <Trash2 className='size-4' />
                        </Button>
                      </div>
                    </div>
                  ))
                )}
              </div>
            </div>
            {pageCount > 1 ? (
              <div className='flex items-center justify-between border-t pt-4 text-sm'>
                <span className='text-muted-foreground'>
                  {t('Page {{page}} of {{total}}', { page, total: pageCount })}
                </span>
                <div className='flex gap-2'>
                  <Button
                    variant='outline'
                    size='sm'
                    disabled={page <= 1}
                    onClick={() => setPage((value) => value - 1)}
                  >
                    {t('Previous')}
                  </Button>
                  <Button
                    variant='outline'
                    size='sm'
                    disabled={page >= pageCount}
                    onClick={() => setPage((value) => value + 1)}
                  >
                    {t('Next')}
                  </Button>
                </div>
              </div>
            ) : null}
          </div>
        </SectionPageLayout.Content>
      </SectionPageLayout>

      <Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
        <SheetContent className='sm:max-w-[520px]'>
          <SheetHeader>
            <SheetTitle>
              {editing ? t('Edit discount code') : t('Create discount code')}
            </SheetTitle>
            <SheetDescription>
              {t(
                'Set a percentage discount. The server checks dates and minimum amount at checkout.'
              )}
            </SheetDescription>
          </SheetHeader>
          <div className='grid gap-5 px-5 py-6'>
            <div className='grid gap-2'>
              <Label htmlFor='discount-form-code'>{t('Code')}</Label>
              <Input
                id='discount-form-code'
                value={form.code}
                onChange={(event) =>
                  setForm((state) => ({
                    ...state,
                    code: event.target.value.toUpperCase(),
                  }))
                }
                maxLength={64}
                autoComplete='off'
              />
            </div>
            <div className='grid gap-2'>
              <Label htmlFor='discount-form-name'>{t('Name')}</Label>
              <Input
                id='discount-form-name'
                value={form.name}
                onChange={(event) =>
                  setForm((state) => ({ ...state, name: event.target.value }))
                }
                maxLength={120}
              />
            </div>
            <div className='grid grid-cols-2 gap-4'>
              <div className='grid gap-2'>
                <Label htmlFor='discount-form-percent'>
                  {t('Discount percent')}
                </Label>
                <Input
                  id='discount-form-percent'
                  type='number'
                  min='1'
                  max='99'
                  value={form.discount_percent}
                  onChange={(event) =>
                    setForm((state) => ({
                      ...state,
                      discount_percent: event.target.value,
                    }))
                  }
                />
              </div>
              <div className='grid gap-2'>
                <Label htmlFor='discount-form-min'>{t('Minimum amount')}</Label>
                <Input
                  id='discount-form-min'
                  type='number'
                  min='0'
                  step='1'
                  value={form.min_amount}
                  onChange={(event) =>
                    setForm((state) => ({
                      ...state,
                      min_amount: event.target.value,
                    }))
                  }
                />
              </div>
            </div>
            <div className='grid gap-2'>
              <Label htmlFor='discount-form-max-uses'>
                {t('Purchase Limit')}
              </Label>
              <Input
                id='discount-form-max-uses'
                type='number'
                min='0'
                step='1'
                value={form.max_uses}
                onChange={(event) =>
                  setForm((state) => ({
                    ...state,
                    max_uses: event.target.value,
                  }))
                }
                aria-invalid={maxUses === undefined}
              />
              <p className='text-muted-foreground text-xs'>
                {t('0 means unlimited')}
              </p>
            </div>
            <div className='grid grid-cols-2 gap-4'>
              <div className='grid gap-2'>
                <Label htmlFor='discount-form-start'>{t('Starts')}</Label>
                <Input
                  id='discount-form-start'
                  type='datetime-local'
                  value={form.starts_time}
                  onChange={(event) =>
                    setForm((state) => ({
                      ...state,
                      starts_time: event.target.value,
                    }))
                  }
                />
              </div>
              <div className='grid gap-2'>
                <Label htmlFor='discount-form-expire'>{t('Expires')}</Label>
                <Input
                  id='discount-form-expire'
                  type='datetime-local'
                  value={form.expired_time}
                  onChange={(event) =>
                    setForm((state) => ({
                      ...state,
                      expired_time: event.target.value,
                    }))
                  }
                />
              </div>
            </div>
          </div>
          <SheetFooter>
            <SheetClose
              render={<Button variant='outline'>{t('Cancel')}</Button>}
            />
            <Button disabled={!canSave || isSaving} onClick={submit}>
              {isSaving ? t('Saving...') : t('Save changes')}
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </>
  )
}
