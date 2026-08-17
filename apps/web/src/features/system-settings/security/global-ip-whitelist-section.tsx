/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import { zodResolver } from '@hookform/resolvers/zod'
import type { TFunction } from 'i18next'
import { useEffect, useMemo, useRef } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import * as z from 'zod'

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
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'

import {
  SettingsForm,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useUpdateOption } from '../hooks/use-update-option'

const createSchema = (t: TFunction) =>
  z
    .object({
      GlobalIPWhitelistEnabled: z.boolean(),
      GlobalIPWhitelistCIDRs: z.string(),
    })
    .superRefine((values, context) => {
      if (
        values.GlobalIPWhitelistEnabled &&
        splitLines(values.GlobalIPWhitelistCIDRs).length === 0
      ) {
        context.addIssue({
          code: 'custom',
          path: ['GlobalIPWhitelistCIDRs'],
          message: t(
            'At least one IP address or CIDR is required before enabling the whitelist.'
          ),
        })
      }
    })

type FormValues = z.infer<ReturnType<typeof createSchema>>

type Props = {
  defaultValues: {
    GlobalIPWhitelistEnabled: boolean
    GlobalIPWhitelistCIDRs: string[]
  }
}

const splitLines = (value: string) =>
  value
    .split('\n')
    .map((entry) => entry.trim())
    .filter(Boolean)

export function GlobalIPWhitelistSection({ defaultValues }: Props) {
  const { t } = useTranslation()
  const updateOption = useUpdateOption()
  const schema = useMemo(() => createSchema(t), [t])
  const baselineRef = useRef(defaultValues)
  const formDefaults = useMemo<FormValues>(
    () => ({
      GlobalIPWhitelistEnabled: defaultValues.GlobalIPWhitelistEnabled,
      GlobalIPWhitelistCIDRs: defaultValues.GlobalIPWhitelistCIDRs.join('\n'),
    }),
    [defaultValues]
  )
  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    mode: 'onChange',
    defaultValues: formDefaults,
  })

  useEffect(() => {
    baselineRef.current = defaultValues
    form.reset(formDefaults)
  }, [defaultValues, form, formDefaults])

  const onSubmit = async (values: FormValues) => {
    const cidrs = splitLines(values.GlobalIPWhitelistCIDRs)
    const baseline = baselineRef.current
    const cidrsChanged =
      JSON.stringify(cidrs) !== JSON.stringify(baseline.GlobalIPWhitelistCIDRs)
    const enabledChanged =
      values.GlobalIPWhitelistEnabled !== baseline.GlobalIPWhitelistEnabled

    if (!cidrsChanged && !enabledChanged) {
      toast.info(t('No changes to save'))
      return
    }

    const saveCIDRs = async () => {
      if (!cidrsChanged) return
      const response = await updateOption.mutateAsync({
        key: 'GlobalIPWhitelistCIDRs',
        value: JSON.stringify(cidrs),
      })
      if (!response.success) throw new Error(response.message)
    }
    const saveEnabled = async () => {
      if (!enabledChanged) return
      const response = await updateOption.mutateAsync({
        key: 'GlobalIPWhitelistEnabled',
        value: values.GlobalIPWhitelistEnabled,
      })
      if (!response.success) throw new Error(response.message)
    }

    if (baseline.GlobalIPWhitelistEnabled && !values.GlobalIPWhitelistEnabled) {
      await saveEnabled()
      await saveCIDRs()
    } else {
      await saveCIDRs()
      await saveEnabled()
    }

    baselineRef.current = {
      GlobalIPWhitelistEnabled: values.GlobalIPWhitelistEnabled,
      GlobalIPWhitelistCIDRs: cidrs,
    }
    form.reset({
      GlobalIPWhitelistEnabled: values.GlobalIPWhitelistEnabled,
      GlobalIPWhitelistCIDRs: cidrs.join('\n'),
    })
  }

  return (
    <SettingsSection title={t('Global IP Whitelist')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={updateOption.isPending}
            saveLabel='Save changes'
          />

          <Alert>
            <AlertTitle>{t('Global ingress allowlist')}</AlertTitle>
            <AlertDescription>
              {t(
                'When enabled, every page and API request except health probes must come from an allowed client IP. Add your current public IP before enabling it.'
              )}
            </AlertDescription>
          </Alert>

          <FormField
            control={form.control}
            name='GlobalIPWhitelistEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable global IP whitelist')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Use trusted-proxy-aware client IP detection and reject all clients outside the list.'
                    )}
                  </FormDescription>
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                  />
                </FormControl>
              </SettingsSwitchItem>
            )}
          />

          <FormField
            control={form.control}
            name='GlobalIPWhitelistCIDRs'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Allowed client IPs / CIDRs')}</FormLabel>
                <FormControl>
                  <Textarea
                    {...field}
                    rows={10}
                    placeholder={'203.0.113.10\n198.51.100.0/24\n2001:db8::/48'}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'Enter one IPv4, IPv6, or CIDR per line. Bare addresses are normalized to exact host prefixes.'
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
