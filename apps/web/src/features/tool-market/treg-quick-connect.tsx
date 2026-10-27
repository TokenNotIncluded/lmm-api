/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'

import { useMarketTranslation as useTranslation } from './provider-i18n'
import type { TregSurface } from './treg-connection'

export function TregQuickConnect({
  surface,
  disabled,
  onSelect,
}: {
  surface?: TregSurface
  disabled: boolean
  onSelect: (surface: TregSurface | undefined) => void
}) {
  const { t } = useTranslation()
  return (
    <Field className='border-b pb-5'>
      <FieldLabel htmlFor='market-treg-surface'>
        {t('treg quick connection')}
      </FieldLabel>
      <select
        id='market-treg-surface'
        aria-describedby='market-treg-surface-help'
        className='border-input bg-background focus-visible:ring-ring min-h-11 w-full rounded-md border px-3 text-base outline-none focus-visible:ring-2 sm:text-sm'
        disabled={disabled}
        value={surface ?? ''}
        onChange={(event) =>
          onSelect(
            event.target.value === 'team' || event.target.value === 'catalog'
              ? event.target.value
              : undefined
          )
        }
      >
        <option value=''>{t('Custom MCP service')}</option>
        <option value='team'>{t('treg — full team MCP')}</option>
        <option value='catalog'>{t('treg — catalog only MCP')}</option>
      </select>
      <FieldDescription id='market-treg-surface-help'>
        {surface === 'catalog'
          ? t(
              'Catalog only: curated endpoints. Media generation and custom team tools are not included.'
            )
          : t(
              'Full team: catalog, media and custom team tools. Choose catalog only when you want the narrower connection.'
            )}
      </FieldDescription>
      {surface && (
        <FieldDescription>
          {t(
            'Use a treg team managed Bearer key. Your LMM sign-in token and treg OAuth tokens are separate credentials.'
          )}
        </FieldDescription>
      )}
    </Field>
  )
}
