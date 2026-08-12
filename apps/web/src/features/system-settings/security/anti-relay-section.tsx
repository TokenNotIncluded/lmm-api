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
import { zodResolver } from '@hookform/resolvers/zod'
import { useEffect, useMemo, useRef, useState } from 'react'
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

const antiRelaySchema = z.object({
  AntiRelayEnabled: z.boolean(),
  AntiRelayRejectProxyHeadersEnabled: z.boolean(),
  AntiRelayHTTPSOnlyEnabled: z.boolean(),
  AntiRelayBlockedCIDRs: z.string(),
  AntiRelayTrustedProxyCIDRs: z.string(),
})

type AntiRelayFormValues = z.infer<typeof antiRelaySchema>

type NormalizedAntiRelayValues = {
  AntiRelayEnabled: boolean
  AntiRelayRejectProxyHeadersEnabled: boolean
  AntiRelayHTTPSOnlyEnabled: boolean
  AntiRelayBlockedCIDRs: string[]
  AntiRelayTrustedProxyCIDRs: string[]
}

type AntiRelaySectionProps = {
  defaultValues: {
    AntiRelayEnabled: boolean
    AntiRelayRejectProxyHeadersEnabled: boolean
    AntiRelayHTTPSOnlyEnabled: boolean
    AntiRelayBlockedCIDRs: string[]
    AntiRelayTrustedProxyCIDRs: string[]
  }
}

const splitLines = (value: string) =>
  value
    .split('\n')
    .map((entry) => entry.trim())
    .filter(Boolean)

const buildFormDefaults = (
  defaults: AntiRelaySectionProps['defaultValues']
): AntiRelayFormValues => ({
  AntiRelayEnabled: defaults.AntiRelayEnabled,
  AntiRelayRejectProxyHeadersEnabled:
    defaults.AntiRelayRejectProxyHeadersEnabled,
  AntiRelayHTTPSOnlyEnabled: defaults.AntiRelayHTTPSOnlyEnabled,
  AntiRelayBlockedCIDRs: formatCIDRs(defaults.AntiRelayBlockedCIDRs),
  AntiRelayTrustedProxyCIDRs: formatCIDRs(defaults.AntiRelayTrustedProxyCIDRs),
})

const normalizeDefaults = (
  defaults: AntiRelaySectionProps['defaultValues']
): NormalizedAntiRelayValues => ({
  AntiRelayEnabled: defaults.AntiRelayEnabled,
  AntiRelayRejectProxyHeadersEnabled:
    defaults.AntiRelayRejectProxyHeadersEnabled,
  AntiRelayHTTPSOnlyEnabled: defaults.AntiRelayHTTPSOnlyEnabled,
  AntiRelayBlockedCIDRs: defaults.AntiRelayBlockedCIDRs,
  AntiRelayTrustedProxyCIDRs: defaults.AntiRelayTrustedProxyCIDRs,
})

const normalizeFormValues = (
  values: AntiRelayFormValues
): NormalizedAntiRelayValues => ({
  AntiRelayEnabled: values.AntiRelayEnabled,
  AntiRelayRejectProxyHeadersEnabled: values.AntiRelayRejectProxyHeadersEnabled,
  AntiRelayHTTPSOnlyEnabled: values.AntiRelayHTTPSOnlyEnabled,
  AntiRelayBlockedCIDRs: splitLines(values.AntiRelayBlockedCIDRs),
  AntiRelayTrustedProxyCIDRs: splitLines(values.AntiRelayTrustedProxyCIDRs),
})

const valuesEqual = (left: unknown, right: unknown) => {
  if (Array.isArray(left) && Array.isArray(right)) {
    return JSON.stringify(left) === JSON.stringify(right)
  }
  return left === right
}

function formatCIDRs(values: string[]) {
  return values.join('\n')
}

export function AntiRelaySection({ defaultValues }: AntiRelaySectionProps) {
  const { t } = useTranslation()
  const updateOption = useUpdateOption()
  const [saveState, setSaveState] = useState<'idle' | 'saved' | 'error'>('idle')
  const baselineRef = useRef<NormalizedAntiRelayValues>(
    normalizeDefaults(defaultValues)
  )
  const formDefaults = useMemo(
    () => buildFormDefaults(defaultValues),
    [defaultValues]
  )
  const form = useForm<AntiRelayFormValues>({
    resolver: zodResolver(antiRelaySchema),
    mode: 'onChange',
    defaultValues: formDefaults,
  })

  useEffect(() => {
    baselineRef.current = normalizeDefaults(defaultValues)
    form.reset(buildFormDefaults(defaultValues))
    setSaveState('idle')
  }, [defaultValues, form])

  const onSubmit = async (values: AntiRelayFormValues) => {
    const normalized = normalizeFormValues(values)
    const baseline = baselineRef.current
    const changedKeys = (
      Object.keys(normalized) as Array<keyof NormalizedAntiRelayValues>
    ).filter((key) => !valuesEqual(normalized[key], baseline[key]))

    if (changedKeys.length === 0) {
      toast.info(t('No changes to save'))
      return
    }

    // Keep the service reachable while an operator turns the policy on or
    // off. Configuration lists are applied before enabling the policy, while
    // disabling happens first.
    const saveOrder: Array<keyof NormalizedAntiRelayValues> = [
      'AntiRelayEnabled',
      'AntiRelayTrustedProxyCIDRs',
      'AntiRelayBlockedCIDRs',
      'AntiRelayRejectProxyHeadersEnabled',
      'AntiRelayHTTPSOnlyEnabled',
    ]
    const disabling = baseline.AntiRelayEnabled && !normalized.AntiRelayEnabled
    const orderedKeys = saveOrder.filter((key) => changedKeys.includes(key))
    if (!disabling) {
      orderedKeys.sort((left, right) => {
        if (left === 'AntiRelayEnabled') return 1
        if (right === 'AntiRelayEnabled') return -1
        return saveOrder.indexOf(left) - saveOrder.indexOf(right)
      })
    }

    setSaveState('idle')
    try {
      for (const key of orderedKeys) {
        const value = normalized[key]
        const response = await updateOption.mutateAsync({
          key,
          value: Array.isArray(value) ? JSON.stringify(value) : value,
        })
        if (!response.success) {
          setSaveState('error')
          return
        }
      }

      baselineRef.current = normalized
      form.reset({
        ...values,
        AntiRelayBlockedCIDRs: formatCIDRs(normalized.AntiRelayBlockedCIDRs),
        AntiRelayTrustedProxyCIDRs: formatCIDRs(
          normalized.AntiRelayTrustedProxyCIDRs
        ),
      })
      setSaveState('saved')
    } catch {
      setSaveState('error')
    }
  }

  return (
    <SettingsSection title={t('Anti-Relay Access')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={updateOption.isPending}
            saveLabel='Save anti-relay settings'
          />

          <Alert>
            <AlertTitle>{t('Anti-relay policy limitation')}</AlertTitle>
            <AlertDescription>
              {t(
                'This is a best-effort ingress policy. It can reject known peer IPs and explicit proxy headers, but no server-side rule can identify every relay that strips or rewrites those signals.'
              )}
            </AlertDescription>
          </Alert>

          {saveState === 'saved' && !form.formState.isDirty ? (
            <p className='text-success text-sm' role='status'>
              {t('Anti-relay settings saved')}
            </p>
          ) : null}
          {saveState === 'error' ? (
            <p className='text-destructive text-sm' role='alert'>
              {t('Anti-relay settings could not be saved')}
            </p>
          ) : null}

          <FormField
            control={form.control}
            name='AntiRelayEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable anti-relay access control')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Reject requests that match a blocked peer IP or expose proxy forwarding signals.'
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
            name='AntiRelayRejectProxyHeadersEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Reject proxy forwarding headers')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Reject Forwarded, Via, X-Forwarded-*, X-Real-IP and similar headers from peers outside the trusted proxy list.'
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
            name='AntiRelayHTTPSOnlyEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Protect HTTPS/443 requests only')}</FormLabel>
                  <FormDescription>
                    {t(
                      'When enabled, apply this policy only to requests identified as HTTPS or port 443. Turn it off to cover every request.'
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
            name='AntiRelayBlockedCIDRs'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Blocked source IPs / CIDRs')}</FormLabel>
                <FormControl>
                  <Textarea
                    {...field}
                    rows={6}
                    placeholder={t(
                      'For example: 198.51.100.10 or 203.0.113.0/24'
                    )}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'One IP or CIDR per line. These are the peer addresses seen by this service, not the client IP claimed by a forwarding header.'
                  )}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />

          <FormField
            control={form.control}
            name='AntiRelayTrustedProxyCIDRs'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Trusted reverse proxy IPs / CIDRs')}</FormLabel>
                <FormControl>
                  <Textarea
                    {...field}
                    rows={6}
                    placeholder={t('For example: 127.0.0.1 or ::1')}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'Add the IPs or CIDRs of your Nginx, CDN, or load balancer as seen by this service. Trusted peers may send forwarding headers and take precedence over blocked CIDRs.'
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
