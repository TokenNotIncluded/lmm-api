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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { SideDrawerSection } from '@/components/drawer-layout'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'

import { getAssistantUserProfile, updateAssistantUserProfile } from '../api'

type AssistantUserProfileEditorProps = {
  userId: number
  open: boolean
}

const PROFILE_OPTIONS = [
  ['guided_buyer', 'Guided setup'],
  ['technical_cost_sensitive', 'Technical and cost sensitive'],
  ['promotion_seeker', 'Promotion seeking'],
  ['security_risk', 'Security sensitive'],
  ['production_operator', 'Production operator'],
  ['privacy_conscious', 'Privacy conscious'],
  ['mobile_accessibility', 'Mobile accessibility'],
  ['support_seeking', 'Support seeking'],
  ['l0_applicant', 'L0 applicant'],
  ['normal_user', 'Normal user'],
  ['custom', 'Custom'],
] as const

export function AssistantUserProfileEditor({
  userId,
  open,
}: AssistantUserProfileEditorProps) {
  const { t } = useTranslation()
  const [profileKey, setProfileKey] = useState('')
  const [tags, setTags] = useState('')
  const [strategy, setStrategy] = useState('')
  const [enabled, setEnabled] = useState(false)
  const [saving, setSaving] = useState(false)

  const profileQuery = useQuery({
    queryKey: ['assistant-user-profile', userId],
    queryFn: () => getAssistantUserProfile(userId),
    enabled: open && userId > 0,
    staleTime: 0,
  })
  const profile = profileQuery.data?.success
    ? profileQuery.data.data
    : undefined
  const profileUnavailable =
    profileQuery.isError || (profileQuery.isFetched && !profile)
  const editorDisabled = profileQuery.isLoading || profileUnavailable

  useEffect(() => {
    if (!profile) return
    setProfileKey(profile.profile_key)
    setTags(profile.tags.join(', '))
    setStrategy(profile.strategy)
    setEnabled(profile.enabled)
  }, [profile])

  const save = async () => {
    setSaving(true)
    try {
      const result = await updateAssistantUserProfile(userId, {
        profile_key: profileKey,
        tags: tags
          .split(',')
          .map((tag) => tag.trim())
          .filter(Boolean),
        strategy,
        enabled,
      })
      if (!result.success) {
        toast.error(result.message || t('Unable to save user profile'))
        return
      }
      toast.success(t('User profile strategy saved'))
      await profileQuery.refetch()
    } catch {
      toast.error(t('Unable to save user profile'))
    } finally {
      setSaving(false)
    }
  }

  return (
    <SideDrawerSection>
      <div className='flex items-center justify-between gap-3'>
        <div>
          <h3 className='text-sm font-medium'>{t('Assistant user profile')}</h3>
          <p className='text-muted-foreground text-xs'>
            {t('Internal moderation guidance used only by the assistant.')}
          </p>
        </div>
        <div className='flex items-center gap-2'>
          <Label htmlFor='assistant-profile-enabled' className='text-xs'>
            {t('Enabled')}
          </Label>
          <Switch
            id='assistant-profile-enabled'
            checked={enabled}
            onCheckedChange={setEnabled}
            disabled={editorDisabled}
          />
        </div>
      </div>

      {profileUnavailable ? (
        <Alert variant='destructive' data-testid='assistant-user-profile-error'>
          <AlertTitle>{t('Failed to load')}</AlertTitle>
          <AlertDescription>{t('Please try again later.')}</AlertDescription>
          <AlertAction>
            <Button
              type='button'
              size='sm'
              variant='outline'
              onClick={() => void profileQuery.refetch()}
            >
              {t('Retry')}
            </Button>
          </AlertAction>
        </Alert>
      ) : null}

      <div className='space-y-2'>
        <Label htmlFor='assistant-profile-key'>{t('Profile tag')}</Label>
        <Select
          items={PROFILE_OPTIONS.map(([value, label]) => ({
            value,
            label: t(label),
          }))}
          value={profileKey}
          onValueChange={(value) => setProfileKey(value ?? '')}
          disabled={editorDisabled}
        >
          <SelectTrigger id='assistant-profile-key'>
            <SelectValue placeholder={t('No manual profile')} />
          </SelectTrigger>
          <SelectContent>
            {PROFILE_OPTIONS.map(([value, label]) => (
              <SelectItem key={value} value={value}>
                {t(label)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className='space-y-2'>
        <Label htmlFor='assistant-profile-tags'>{t('Profile labels')}</Label>
        <Input
          id='assistant-profile-tags'
          value={tags}
          onChange={(event) => setTags(event.target.value)}
          placeholder={t('Separate labels with commas')}
          disabled={editorDisabled}
        />
      </div>

      <div className='space-y-2'>
        <Label htmlFor='assistant-profile-strategy'>
          {t('Manual handling strategy')}
        </Label>
        <Textarea
          id='assistant-profile-strategy'
          value={strategy}
          onChange={(event) => setStrategy(event.target.value)}
          placeholder={t(
            'Describe how the assistant should respond to this user.'
          )}
          rows={4}
          disabled={editorDisabled}
        />
        <p className='text-muted-foreground text-xs'>
          {t(
            'This strategy is hidden from the user and must not contain secrets.'
          )}
        </p>
      </div>

      <Button
        type='button'
        variant='outline'
        onClick={save}
        disabled={saving || editorDisabled}
        data-testid='assistant-user-profile-save'
      >
        {saving
          ? t('Saving...')
          : profileQuery.isLoading
            ? t('Loading')
            : t('Save profile strategy')}
      </Button>
    </SideDrawerSection>
  )
}
