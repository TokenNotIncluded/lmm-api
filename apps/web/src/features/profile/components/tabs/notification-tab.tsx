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
import { Check, Eye, EyeOff, Loader2, UserRound } from 'lucide-react'
import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'

import { updateUserSettings } from '../../api'
import { parseUserSettings } from '../../lib'
import type {
  UserProfile,
  UserSettings,
  UsageLeaderboardVisibility,
} from '../../types'
import { NotificationPreferencesForm } from '../notification-preferences'

const USAGE_LEADERBOARD_VISIBILITY_OPTIONS = [
  { value: 'public', label: 'Show my name', icon: UserRound },
  { value: 'anonymous', label: 'Show anonymously', icon: Eye },
  { value: 'hidden', label: 'Do not show me', icon: EyeOff },
] as const

const USAGE_LEADERBOARD_VISIBILITY_VALUES = new Set<UsageLeaderboardVisibility>(
  USAGE_LEADERBOARD_VISIBILITY_OPTIONS.map((option) => option.value)
)

function normalizeUsageLeaderboardVisibility(
  value: unknown
): UsageLeaderboardVisibility {
  return typeof value === 'string' &&
    USAGE_LEADERBOARD_VISIBILITY_VALUES.has(value as UsageLeaderboardVisibility)
    ? (value as UsageLeaderboardVisibility)
    : 'anonymous'
}

// ============================================================================
// Settings Tab Component
// ============================================================================

interface NotificationTabProps {
  profile: UserProfile | null
  onUpdate: () => void
}

export function NotificationTab({ profile, onUpdate }: NotificationTabProps) {
  const { t } = useTranslation()
  const isL1OrAbove = (profile?.trust_level_info?.level ?? 0) >= 1
  const [loading, setLoading] = useState(false)
  const [settings, setSettings] = useState<UserSettings>({
    accept_unset_model_ratio_model: false,
    record_ip_log: false,
    usage_leaderboard_visibility: 'anonymous',
    allow_key_bypass_ip_policy: false,
  })

  // Update form field helper
  const updateField = useCallback(
    <K extends keyof UserSettings>(field: K, value: UserSettings[K]) => {
      setSettings((prev) => ({ ...prev, [field]: value }))
    },
    []
  )

  useEffect(() => {
    if (profile?.setting) {
      const parsed = parseUserSettings(profile.setting)
      setSettings({
        accept_unset_model_ratio_model:
          parsed.accept_unset_model_ratio_model || false,
        record_ip_log: parsed.record_ip_log || false,
        usage_leaderboard_visibility: normalizeUsageLeaderboardVisibility(
          parsed.usage_leaderboard_visibility
        ),
        allow_key_bypass_ip_policy: parsed.allow_key_bypass_ip_policy || false,
      })
    }
  }, [profile])

  const handleSave = async () => {
    try {
      setLoading(true)
      const response = await updateUserSettings({
        accept_unset_model_ratio_model: settings.accept_unset_model_ratio_model,
        record_ip_log: settings.record_ip_log,
        usage_leaderboard_visibility: settings.usage_leaderboard_visibility,
        allow_key_bypass_ip_policy: settings.allow_key_bypass_ip_policy,
      })

      if (response.success) {
        toast.success(t('Settings updated successfully'))
        onUpdate()
      } else {
        toast.error(response.message || t('Failed to update settings'))
      }
    } catch {
      toast.error(t('Failed to update settings'))
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className='space-y-4 sm:space-y-6'>
      <NotificationPreferencesForm userId={profile?.id} onUpdate={onUpdate} />

      {/* Divider */}
      <div className='border-t' />

      {/* Preferences Section */}
      <div className='space-y-3'>
        <div>
          <h4 className='text-sm font-medium'>{t('Preferences')}</h4>
          <p className='text-muted-foreground mt-1 text-xs'>
            {t('Configure your account behavior preferences')}
          </p>
        </div>

        {/* Public Usage Leaderboard */}
        <div className='space-y-2.5 rounded-none border p-3 sm:p-4'>
          <div>
            <Label>{t('Usage leaderboard visibility')}</Label>
            <p className='text-muted-foreground mt-1 text-xs sm:text-sm'>
              {t(
                'Choose how your usage appears on the public leaderboard. Default is anonymous.'
              )}
            </p>
          </div>
          <ToggleGroup
            value={[
              normalizeUsageLeaderboardVisibility(
                settings.usage_leaderboard_visibility
              ),
            ]}
            onValueChange={(value) => {
              const nextValue = value.find(
                (item) =>
                  item !== settings.usage_leaderboard_visibility &&
                  USAGE_LEADERBOARD_VISIBILITY_VALUES.has(
                    item as UsageLeaderboardVisibility
                  )
              )
              if (nextValue) {
                updateField(
                  'usage_leaderboard_visibility',
                  normalizeUsageLeaderboardVisibility(nextValue)
                )
              }
            }}
            aria-label={t('Usage leaderboard visibility')}
            variant='outline'
            size='lg'
            spacing={2}
            className='grid w-full grid-cols-1 gap-2 sm:grid-cols-3'
          >
            {USAGE_LEADERBOARD_VISIBILITY_OPTIONS.map((option) => {
              const Icon = option.icon
              const isSelected =
                normalizeUsageLeaderboardVisibility(
                  settings.usage_leaderboard_visibility
                ) === option.value
              return (
                <ToggleGroupItem
                  key={option.value}
                  value={option.value}
                  aria-pressed={isSelected}
                  className={cn(
                    'h-auto min-h-14 w-full justify-between gap-2 px-3 py-3 text-xs font-medium transition-[transform,background-color,border-color,box-shadow] duration-150 active:scale-[0.99] sm:min-h-16 sm:text-sm',
                    isSelected &&
                      'border-primary bg-primary/10 text-primary shadow-sm ring-1 ring-primary/30 data-[state=on]:bg-primary/10'
                  )}
                >
                  <span className='flex min-w-0 items-center gap-2'>
                    <Icon className='h-4 w-4 shrink-0' />
                    <span className='truncate'>{t(option.label)}</span>
                  </span>
                  {isSelected ? (
                    <Badge
                      variant='secondary'
                      className='shrink-0 gap-1 px-2 py-0.5 text-[11px] font-medium'
                    >
                      <Check className='size-3.5' aria-hidden='true' />
                      {t('Current')}
                    </Badge>
                  ) : null}
                </ToggleGroupItem>
              )
            })}
          </ToggleGroup>
          <p className='text-muted-foreground text-xs'>
            {t('Your choice controls whether your usage is listed publicly.')}
          </p>
        </div>

        {/* Accept Unset Model Price */}
        <div className='flex items-start justify-between gap-3 rounded-none border p-3 sm:items-center sm:p-4'>
          <div className='space-y-0.5'>
            <Label htmlFor='acceptUnsetPrice'>
              {t('Accept Unpriced Models')}
            </Label>
            <p className='text-muted-foreground text-xs sm:text-sm'>
              {t('Allow using models without price configuration')}
            </p>
          </div>
          <Switch
            id='acceptUnsetPrice'
            className='shrink-0'
            checked={settings.accept_unset_model_ratio_model}
            onCheckedChange={(checked) =>
              updateField('accept_unset_model_ratio_model', checked)
            }
          />
        </div>

        {/* Record IP Log */}
        <div className='flex items-start justify-between gap-3 rounded-none border p-3 sm:items-center sm:p-4'>
          <div className='space-y-0.5'>
            <Label htmlFor='recordIp'>{t('Record IP Address')}</Label>
            <p className='text-muted-foreground text-xs sm:text-sm'>
              {t('Log IP address for usage and error logs')}
            </p>
          </div>
          <Switch
            id='recordIp'
            className='shrink-0'
            checked={settings.record_ip_log}
            onCheckedChange={(checked) => updateField('record_ip_log', checked)}
          />
        </div>

        {/* Bypass IP/Region Policy With API Key (L1+ only) */}
        {isL1OrAbove && (
          <div className='flex items-start justify-between gap-3 rounded-none border p-3 sm:items-center sm:p-4'>
            <div className='space-y-0.5'>
              <Label htmlFor='allowKeyBypassIpPolicy'>
                {t('Bypass IP/Region Policy With API Key')}
              </Label>
              <p className='text-muted-foreground line-clamp-3 text-xs sm:line-clamp-none sm:text-sm'>
                {t(
                  'Available from trust level L1. When enabled, requests that include a valid API key for your account skip IP/region access restrictions. Requests without a valid key are unaffected.'
                )}
              </p>
            </div>
            <Switch
              id='allowKeyBypassIpPolicy'
              className='shrink-0'
              checked={settings.allow_key_bypass_ip_policy}
              onCheckedChange={(checked) =>
                updateField('allow_key_bypass_ip_policy', checked)
              }
            />
          </div>
        )}
      </div>

      {/* Save Button */}
      <div className='flex justify-end'>
        <Button onClick={handleSave} disabled={loading}>
          {loading && <Loader2 className='mr-2 h-4 w-4 animate-spin' />}
          {loading ? t('Saving...') : t('Save Settings')}
        </Button>
      </div>
    </div>
  )
}
