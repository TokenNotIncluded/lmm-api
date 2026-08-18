/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Textarea } from '@/components/ui/textarea'

import { updateSystemOption, validateSystemOptions } from '../api'
import { SettingsForm } from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import {
  createIPAccessRoutingSchema,
  DEFAULT_IP_ACCESS_ROUTING_RULES,
  type IPAccessRoutingFormValues,
  normalizeIPAccessRoutingRules,
} from './ip-access-routing-config'

type Props = {
  defaultValues: {
    IPAccessRoutingRules: string
  }
}

export function IPAccessRoutingSection({ defaultValues }: Props) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const schema = useMemo(() => createIPAccessRoutingSchema(t), [t])
  const initialRules = normalizeIPAccessRoutingRules(
    defaultValues.IPAccessRoutingRules
  )
  const baselineRef = useRef(initialRules)
  const form = useForm<IPAccessRoutingFormValues>({
    resolver: zodResolver(schema),
    mode: 'onChange',
    defaultValues: { IPAccessRoutingRules: initialRules },
  })
  const saveRules = useMutation({
    mutationFn: async (rules: string) => {
      const validation = await validateSystemOptions({
        IPAccessRoutingRules: rules,
      })
      if (!validation.success) {
        throw new Error(validation.message || t('Routing rules are invalid.'))
      }

      const response = await updateSystemOption({
        key: 'IPAccessRoutingRules',
        value: rules,
      })
      if (!response.success) {
        throw new Error(response.message || t('Failed to update setting'))
      }
    },
  })

  useEffect(() => {
    baselineRef.current = initialRules
    form.reset({ IPAccessRoutingRules: initialRules })
  }, [form, initialRules])

  const onSubmit = async (values: IPAccessRoutingFormValues) => {
    const rules = normalizeIPAccessRoutingRules(values.IPAccessRoutingRules)
    if (rules === baselineRef.current) {
      toast.info(t('No changes to save'))
      return
    }

    form.clearErrors('IPAccessRoutingRules')
    try {
      await saveRules.mutateAsync(rules)
      baselineRef.current = rules
      form.reset({ IPAccessRoutingRules: rules })
      await queryClient.invalidateQueries({ queryKey: ['system-options'] })
      toast.success(t('Setting updated successfully'))
    } catch (error) {
      const message =
        error instanceof Error ? error.message : t('Failed to update setting')
      form.setError('IPAccessRoutingRules', { type: 'server', message })
      toast.error(message)
    }
  }

  return (
    <SettingsSection title={t('IP & Region Routing')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={saveRules.isPending}
            saveLabel='Save changes'
          />

          <Alert>
            <AlertTitle>{t('First matching rule wins')}</AlertTitle>
            <AlertDescription>
              {t(
                'Rules run from top to bottom; direct allows and reject blocks. Add fallback: direct or fallback: reject to set the default for unmatched requests; without it, unmatched requests use direct.'
              )}
            </AlertDescription>
          </Alert>

          <Alert variant='destructive'>
            <AlertTitle>{t('Keep management access first')}</AlertTitle>
            <AlertDescription>
              {t(
                'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.'
              )}
            </AlertDescription>
          </Alert>

          <FormField
            control={form.control}
            name='IPAccessRoutingRules'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Routing rules')}</FormLabel>
                <FormControl>
                  <Textarea
                    {...field}
                    className='min-h-72 resize-y font-mono text-sm leading-6'
                    rows={14}
                    autoCapitalize='none'
                    autoCorrect='off'
                    spellCheck={false}
                    placeholder={DEFAULT_IP_ACCESS_ROUTING_RULES}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'Use Daed routing syntax. Matchers: domain/qname, dip/ip, sip, dport, sport, l4proto, ipversion, mac, pname, and dscp; supports ! negation, fallback, and direct/reject. Use # for comments.'
                  )}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
        </SettingsForm>
      </Form>
    </SettingsSection>
  )
}
