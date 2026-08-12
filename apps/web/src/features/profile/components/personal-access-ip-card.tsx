/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import { Globe2, Info, Loader2, ShieldCheck, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { TitledCard } from '@/components/ui/titled-card'

import {
  deletePersonalAccessIP,
  getPersonalAccessIP,
  setPersonalAccessIP,
} from '../api'
import type { UserProfile } from '../types'

interface PersonalAccessIPCardProps {
  profile: UserProfile | null
  loading: boolean
}

const MIN_TRUST_LEVEL = 2

export function PersonalAccessIPCard({
  profile,
  loading: pageLoading,
}: PersonalAccessIPCardProps) {
  const { t } = useTranslation()
  const trustLevel = profile?.trust_level_info?.level ?? 0
  const eligible = trustLevel >= MIN_TRUST_LEVEL
  const [policyIP, setPolicyIP] = useState('')
  const [currentIP, setCurrentIP] = useState('')
  const [inputIP, setInputIP] = useState('')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!eligible || pageLoading) return
    let cancelled = false
    setLoading(true)
    void getPersonalAccessIP()
      .then((response) => {
        if (cancelled) return
        if (!response.success || !response.data) {
          toast.error(response.message || t('Failed to load IP policy'))
          return
        }
        setPolicyIP(response.data.ip || '')
        setInputIP(response.data.ip || '')
        setCurrentIP(response.data.current_ip || '')
      })
      .catch(() => {
        if (!cancelled) toast.error(t('Failed to load IP policy'))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [eligible, pageLoading, t])

  const handleSave = async () => {
    const nextIP = inputIP.trim()
    if (!nextIP) {
      toast.error(t('Enter a public IP address'))
      return
    }
    setSaving(true)
    try {
      const response = await setPersonalAccessIP(nextIP)
      if (!response.success || !response.data) {
        toast.error(response.message || t('Failed to save IP policy'))
        return
      }
      setPolicyIP(response.data.ip || nextIP)
      setInputIP(response.data.ip || nextIP)
      setCurrentIP(response.data.current_ip || currentIP)
      toast.success(t('IP allowlist saved'))
    } catch {
      toast.error(t('Failed to save IP policy'))
    } finally {
      setSaving(false)
    }
  }

  const handleClear = async () => {
    setSaving(true)
    try {
      const response = await deletePersonalAccessIP()
      if (!response.success) {
        toast.error(response.message || t('Failed to clear IP policy'))
        return
      }
      setPolicyIP('')
      setInputIP('')
      toast.success(t('IP allowlist cleared'))
    } catch {
      toast.error(t('Failed to clear IP policy'))
    } finally {
      setSaving(false)
    }
  }

  if (pageLoading) return null

  let content
  if (!eligible) {
    content = (
      <div className='bg-muted/30 flex items-start gap-3 rounded-lg border p-3'>
        <ShieldCheck className='text-muted-foreground mt-0.5 size-4 shrink-0' />
        <div className='min-w-0'>
          <p className='text-sm font-medium'>{t('Unlocks at L1')}</p>
          <p className='text-muted-foreground mt-1 text-xs leading-5'>
            {t(
              'Reach trust level L1 to register one public IP address for direct access.'
            )}
          </p>
        </div>
      </div>
    )
  } else if (loading) {
    content = (
      <div className='text-muted-foreground flex items-center gap-2 text-sm'>
        <Loader2 className='size-4 animate-spin' />
        {t('Loading')}
      </div>
    )
  } else {
    content = (
      <div className='space-y-4'>
        <div className='border-primary/20 bg-primary/5 rounded-lg border p-3'>
          <div className='flex items-start gap-3'>
            <Info className='text-primary mt-0.5 size-4 shrink-0' />
            <div className='min-w-0 space-y-1'>
              <p className='text-sm font-medium'>
                {t('How this direct-access exception works')}
              </p>
              <p className='text-muted-foreground text-xs leading-5'>
                {t(
                  "The production edge checks the request's exit IP. A mainland-China exit IP must match your registered address; other regions continue normally."
                )}
              </p>
              <p className='text-muted-foreground text-xs leading-5'>
                {t(
                  'This is not a proxy or relay block. It does not prevent model requests made through a proxy; only the proxy exit IP is evaluated.'
                )}
              </p>
            </div>
          </div>
        </div>

        <div className='bg-muted/20 rounded-lg border p-3'>
          <div className='flex flex-wrap items-center justify-between gap-2'>
            <span className='text-muted-foreground text-xs'>
              {t('Detected public IP for this request')}
            </span>
            <code className='text-sm'>{currentIP || '—'}</code>
          </div>
          <p className='text-muted-foreground mt-1 text-xs leading-5'>
            {t(
              'Only you can see this address. It is used for access matching and is not shown to other users.'
            )}
          </p>
        </div>

        <div className='flex flex-col gap-2 sm:flex-row'>
          <Input
            value={inputIP}
            onChange={(event) => setInputIP(event.target.value)}
            placeholder={t('Public IP for direct access')}
            aria-label={t('Public IP for direct access')}
            disabled={saving}
          />
          <Button
            type='button'
            onClick={handleSave}
            disabled={saving || !inputIP.trim() || inputIP.trim() === policyIP}
            className='w-full sm:w-auto'
          >
            {saving && <Loader2 className='animate-spin' />}
            {t('Save')}
          </Button>
        </div>

        {policyIP && (
          <div className='flex flex-wrap items-center justify-between gap-2 text-xs'>
            <span className='text-muted-foreground'>
              {t('Registered direct-access address')}
            </span>
            <code>{policyIP}</code>
          </div>
        )}

        <div className='flex flex-wrap items-center justify-between gap-2 text-xs'>
          <span className='text-muted-foreground'>
            {t('Only one address can be registered')}
          </span>
          {policyIP && (
            <Button
              type='button'
              variant='ghost'
              size='sm'
              onClick={handleClear}
              disabled={saving}
              className='text-destructive hover:text-destructive h-7 px-2'
            >
              <Trash2 data-icon='inline-start' />
              {t('Clear')}
            </Button>
          )}
        </div>
        <p className='text-muted-foreground text-xs leading-5'>
          {t(
            'If you use a VPN or proxy, enter its exit IP instead of your local address.'
          )}
        </p>
      </div>
    )
  }

  return (
    <TitledCard
      title={t('Direct access exception')}
      description={t(
        'Register one public IP for direct access to the site and API.'
      )}
      icon={<Globe2 className='h-4 w-4' />}
      iconTone={eligible ? 'success' : 'neutral'}
      disableHoverEffect
    >
      {content}
    </TitledCard>
  )
}
