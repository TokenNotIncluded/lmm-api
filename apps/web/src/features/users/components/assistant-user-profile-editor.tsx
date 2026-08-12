import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { SideDrawerSection } from '@/components/drawer-layout'
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

  useEffect(() => {
    const profile = profileQuery.data?.data
    if (!profile) return
    setProfileKey(profile.profile_key)
    setTags(profile.tags.join(', '))
    setStrategy(profile.strategy)
    setEnabled(profile.enabled)
  }, [profileQuery.data])

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
          />
        </div>
      </div>

      <div className='space-y-2'>
        <Label htmlFor='assistant-profile-key'>{t('Profile tag')}</Label>
        <Select
          items={PROFILE_OPTIONS.map(([value, label]) => ({
            value,
            label: t(label),
          }))}
          value={profileKey}
          onValueChange={(value) => setProfileKey(value ?? '')}
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
        />
        <p className='text-muted-foreground text-xs'>
          {t(
            'This strategy is hidden from the user and must not contain secrets.'
          )}
        </p>
      </div>

      <Button type='button' variant='outline' onClick={save} disabled={saving}>
        {saving ? t('Saving...') : t('Save profile strategy')}
      </Button>
    </SideDrawerSection>
  )
}
