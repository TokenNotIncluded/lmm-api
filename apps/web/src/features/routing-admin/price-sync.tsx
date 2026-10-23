/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useId, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

import {
  download,
  errorMessage,
  exportPrices,
  operationId,
  pricePreview,
  publish,
  readSourceFile,
} from './api'
import type { Adjustment, Preview } from './contract'
import { adjustmentLabels } from './helpers'
import { PublicationReview } from './publication-review'
import { useCatalog } from './use-catalog'

export function NativePriceSync() {
  const { t } = useTranslation(),
    { scope, catalog, error: loadError, loading, reload } = useCatalog(),
    fileId = useId()
  const [source, setSource] = useState(''),
    [sourceName, setSourceName] = useState(''),
    [selections, setSelections] = useState<Record<string, Adjustment>>({}),
    [bulk, setBulk] = useState<Adjustment>('keep'),
    [preview, setPreview] = useState<Preview>(),
    [operation, setOperation] = useState(''),
    [error, setError] = useState<unknown>(),
    [busy, setBusy] = useState(false),
    [status, setStatus] = useState('')
  const models = useMemo(
    () => [...new Set(catalog?.prices.map((p) => p.model) ?? [])].sort(),
    [catalog]
  )
  useEffect(() => {
    setPreview(undefined)
    setSelections({})
    setSource('')
    setSourceName('')
    setStatus('')
  }, [scope])
  function invalidate() {
    setPreview(undefined)
    setError(undefined)
    setStatus('')
  }
  async function prepare() {
    if (!scope || !catalog) return
    setBusy(true)
    setError(undefined)
    setStatus('')
    try {
      const value = await pricePreview(
        scope,
        catalog.expected,
        source,
        Object.entries(selections).map(([model, adjustment]) => ({
          model,
          adjustment,
        }))
      )
      setPreview(value)
      setOperation(operationId())
    } catch (error) {
      setError(error)
    } finally {
      setBusy(false)
    }
  }
  async function confirm() {
    if (!scope || !preview) return
    setBusy(true)
    setError(undefined)
    try {
      const result = await publish(scope, preview, operation)
      setPreview(undefined)
      setSelections({})
      setStatus(
        result.applied_on_this_node
          ? 'Prices published successfully.'
          : 'Published successfully. Requests will resume when the new configuration is ready.'
      )
      await reload()
    } catch (error) {
      setError(error)
      setStatus(
        'Publication could not be confirmed. Retry to check the same operation.'
      )
    } finally {
      setBusy(false)
    }
  }
  const choices = (
    <>
      {Object.entries(adjustmentLabels).map(([value, label]) => (
        <NativeSelectOption
          key={value}
          value={value}
          disabled={value !== 'keep' && !catalog?.exchange_rate_available}
        >
          {t(label)}
        </NativeSelectOption>
      ))}
    </>
  )
  return (
    <section
      className='space-y-5'
      aria-label={t('Native price synchronization')}
    >
      <p className='text-muted-foreground text-sm'>
        {t(
          'Import Core native price source v1 files in USD. Existing model, tier, cache, and search metering contracts must match.'
        )}
      </p>
      {Boolean(loadError) && (
        <p role='alert' className='text-destructive text-sm'>
          {t(errorMessage(loadError))}
        </p>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button
          variant='outline'
          disabled={loading || busy}
          onClick={() => {
            invalidate()
            void reload()
          }}
        >
          {loading ? t('Loading...') : t('Reload')}
        </Button>
        <Button
          variant='outline'
          disabled={busy || !scope || !catalog?.can_edit_prices}
          onClick={async () => {
            if (!scope) return
            setBusy(true)
            setError(undefined)
            try {
              download(await exportPrices(scope), 'core-native-prices-v1.json')
            } catch (error) {
              setError(error)
            } finally {
              setBusy(false)
            }
          }}
        >
          {t('Export native prices')}
        </Button>
      </div>
      {catalog && !catalog.can_edit_prices && (
        <p role='status'>
          {t('You do not have permission to publish this change.')}
        </p>
      )}
      {catalog?.can_edit_prices && (
        <>
          <div className='space-y-2'>
            <Label htmlFor={fileId}>{t('Native price source file')}</Label>
            <Input
              id={fileId}
              type='file'
              accept='.json,application/json'
              disabled={busy}
              onChange={async (event) => {
                invalidate()
                const file = event.currentTarget.files?.[0]
                setSource('')
                setSourceName('')
                if (!file) return
                setBusy(true)
                try {
                  setSource(await readSourceFile(file))
                  setSourceName(file.name)
                } catch (error) {
                  setError(error)
                } finally {
                  setBusy(false)
                }
              }}
            />
            {sourceName && <p className='text-sm'>{sourceName}</p>}
          </div>
          <p className='text-sm'>
            {t('Site exchange rate')}:{' '}
            {catalog.exchange_rate_available
              ? catalog.exchange_rate?.raw
              : t('Unavailable')}
          </p>
          <div className='flex flex-wrap items-center gap-2'>
            <NativeSelect
              aria-label={t('Adjustment for selected models')}
              value={bulk}
              disabled={busy}
              onChange={(e) => setBulk(e.target.value as Adjustment)}
            >
              {choices}
            </NativeSelect>
            <Button
              variant='outline'
              disabled={busy || Object.keys(selections).length === 0}
              onClick={() => {
                invalidate()
                setSelections(
                  Object.fromEntries(
                    Object.keys(selections).map((model) => [model, bulk])
                  )
                )
              }}
            >
              {t('Apply to selected')}
            </Button>
            <Button
              variant='ghost'
              disabled={busy}
              onClick={() => {
                invalidate()
                setSelections({})
              }}
            >
              {t('Clear selection')}
            </Button>
          </div>
          <div className='max-h-96 overflow-auto rounded-xl border'>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t('Select')}</TableHead>
                  <TableHead>{t('Model')}</TableHead>
                  <TableHead>{t('Price adjustment')}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {models.map((model) => {
                  const selected = Object.hasOwn(selections, model),
                    supported = catalog.prices
                      .filter((p) => p.model === model)
                      .every((p) => p.billing_mode !== 'upstream_cost')
                  return (
                    <TableRow key={model}>
                      <TableCell>
                        <Checkbox
                          aria-label={t('Select {{model}}', { model })}
                          disabled={
                            busy ||
                            !supported ||
                            (!selected && Object.keys(selections).length >= 200)
                          }
                          checked={selected}
                          onCheckedChange={(checked) => {
                            invalidate()
                            setSelections((current) => {
                              const next = { ...current }
                              if (checked) next[model] = 'keep'
                              else delete next[model]
                              return next
                            })
                          }}
                        />
                      </TableCell>
                      <TableCell>
                        <span className='font-medium'>{model}</span>
                        {!supported && (
                          <span className='text-muted-foreground block text-xs'>
                            {t('Upstream cost pricing cannot be imported.')}
                          </span>
                        )}
                      </TableCell>
                      <TableCell>
                        <NativeSelect
                          aria-label={t('Price adjustment for {{model}}', {
                            model,
                          })}
                          disabled={busy || !selected}
                          value={selections[model] ?? 'keep'}
                          onChange={(e) => {
                            invalidate()
                            setSelections((current) => ({
                              ...current,
                              [model]: e.target.value as Adjustment,
                            }))
                          }}
                        >
                          {choices}
                        </NativeSelect>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          </div>
          <p className='text-muted-foreground text-xs'>
            {t(
              'Select up to 200 models. Every group and speed for a selected model is included.'
            )}
          </p>
          <Button
            disabled={busy || !source || Object.keys(selections).length === 0}
            onClick={() => void prepare()}
          >
            {t('Preview adjusted prices')}
          </Button>
        </>
      )}
      {Boolean(error) && (
        <p role='alert' className='text-destructive text-sm'>
          {t(errorMessage(error))}
        </p>
      )}
      {status && (
        <p role='status' className='text-sm'>
          {t(status)}
        </p>
      )}
      {preview && (
        <PublicationReview
          preview={preview}
          busy={busy}
          onConfirm={() => void confirm()}
          onCancel={() => setPreview(undefined)}
        />
      )}
    </section>
  )
}
