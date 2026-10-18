/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useAuthStore } from '@/stores/auth-store'

import { isCurrentNativeUsersScope, type NativeUsersScope } from './access'
import {
  NativeProfileFailure,
  readNativeProfile,
  readNativeProfileCapabilities,
  updateNativeDisplayName,
} from './profile-api'
import {
  hasNativeDisplayNameControls,
  validNativeDisplayName,
  type NativeProfile,
} from './profile-contract'

type Props = {
  scope: NativeUsersScope
  userId: string
  onUpdated: () => void
  onAccessDenied: () => void
}
type Notice = 'none' | 'unavailable' | 'conflict' | 'unknown' | 'invalid'

export function NativeDisplayNameEditor({
  scope,
  userId,
  onUpdated,
  onAccessDenied,
}: Props) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const stableScope = useMemo(
    () => ({
      userId: scope.userId,
      sessionId: scope.sessionId,
      role: scope.role,
    }),
    [scope.userId, scope.sessionId, scope.role]
  )
  const [open, setOpen] = useState(false)
  const [profile, setProfile] = useState<NativeProfile>()
  const [draft, setDraft] = useState('')
  const [phase, setPhase] = useState<'loading' | 'ready' | 'failed' | 'saving'>(
    'loading'
  )
  const [notice, setNotice] = useState<Notice>('none')
  const [unresolvedWrite, setUnresolvedWrite] = useState(false)
  const [revision, setRevision] = useState(0)
  const preserveDraft = useRef(false)
  const busy = useRef(false)
  const holder = useRef({
    generation: 0,
    active: false,
    controller: undefined as AbortController | undefined,
  })
  const callbacks = useRef({ onUpdated, onAccessDenied })
  callbacks.current = { onUpdated, onAccessDenied }
  const current = isCurrentNativeUsersScope(stableScope)

  const clear = useCallback(() => {
    holder.current.generation++
    holder.current.controller?.abort()
    setOpen(false)
    setProfile(undefined)
    setDraft('')
    setNotice('none')
    setUnresolvedWrite(false)
  }, [])

  useEffect(() => {
    if (!isCurrentNativeUsersScope(stableScope)) clear()
  }, [auth.user?.id, auth.session?.sid, auth.user?.role, stableScope, clear])

  useEffect(() => {
    if (!open || !isCurrentNativeUsersScope(stableScope)) return
    const lifecycle = holder.current
    lifecycle.active = true
    const generation = ++lifecycle.generation
    const controller = new AbortController()
    lifecycle.controller = controller
    const keepDraft = preserveDraft.current
    const owns = () =>
      lifecycle.active &&
      lifecycle.generation === generation &&
      isCurrentNativeUsersScope(stableScope) &&
      !controller.signal.aborted
    setPhase('loading')
    setProfile(undefined)
    if (!keepDraft) {
      setDraft('')
      setNotice('none')
      setUnresolvedWrite(false)
    }
    void (async () => {
      try {
        await readNativeProfileCapabilities(stableScope, controller.signal)
        const loaded = await readNativeProfile(
          stableScope,
          userId,
          controller.signal
        )
        if (!owns()) return
        setProfile(loaded)
        if (!keepDraft) setDraft(loaded.display_name)
        // A read supplies fresh CAS material. It never confirms an earlier write.
        setNotice((previous) => (previous === 'unknown' ? 'unknown' : 'none'))
        setPhase('ready')
      } catch (error) {
        if (!owns()) return
        if (
          error instanceof NativeProfileFailure &&
          error.kind === 'forbidden'
        ) {
          clear()
          callbacks.current.onAccessDenied()
          return
        }
        setNotice((previous) =>
          previous === 'unknown' ? 'unknown' : 'unavailable'
        )
        setPhase('failed')
      }
    })()
    return () => {
      lifecycle.active = false
      lifecycle.generation++
      controller.abort()
    }
  }, [open, revision, stableScope, userId, clear])

  const save = async (event: FormEvent) => {
    event.preventDefault()
    if (
      busy.current ||
      phase !== 'ready' ||
      !profile ||
      !validNativeDisplayName(draft) ||
      !isCurrentNativeUsersScope(stableScope) ||
      (unresolvedWrite && draft === profile.display_name)
    ) {
      return
    }
    const generation = holder.current.generation
    const submitted = draft
    const before = profile
    const owns = () =>
      holder.current.active &&
      holder.current.generation === generation &&
      isCurrentNativeUsersScope(stableScope)
    busy.current = true
    setPhase('saving')
    try {
      const result = await updateNativeDisplayName(
        stableScope,
        before,
        submitted
      )
      if (!owns()) return
      if (result.changed) toast.success(t('Display name changed successfully'))
      else toast.info(t('No changes'))
      clear()
      callbacks.current.onUpdated()
    } catch (error) {
      if (!owns()) return
      const kind =
        error instanceof NativeProfileFailure ? error.kind : 'unknown'
      if (kind === 'forbidden' || kind === 'session_changed') {
        clear()
        if (kind === 'forbidden') callbacks.current.onAccessDenied()
        return
      }
      // No old CAS can be resubmitted after conflict, uncertainty or a refusal.
      setProfile(undefined)
      setPhase('failed')
      setNotice(
        kind === 'conflict'
          ? 'conflict'
          : kind === 'invalid'
            ? 'invalid'
            : kind === 'not_found'
              ? 'unavailable'
              : 'unknown'
      )
      if (kind !== 'conflict' && kind !== 'invalid' && kind !== 'not_found') {
        setUnresolvedWrite(true)
      }
    } finally {
      busy.current = false
    }
  }

  if (!current) return null
  const invalid = !validNativeDisplayName(draft)
  const disabled =
    phase !== 'ready' ||
    !profile ||
    invalid ||
    (unresolvedWrite && draft === profile.display_name)
  const fieldId = `native-display-name-${userId}`
  return (
    <>
      <Button
        type='button'
        variant='outline'
        className='min-h-11 sm:min-h-9'
        aria-label={`${t('Edit')}: ${t('Display name')} #${userId}`}
        onClick={() => {
          preserveDraft.current = false
          setOpen(true)
        }}
      >
        {t('Edit')} · {t('Display name')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!value) clear()
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('Display name')}</DialogTitle>
            <DialogDescription>
              {t('User ID')}:{' '}
              <span className='font-mono break-all'>{userId}</span>
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void save(event)} className='space-y-4'>
            {phase === 'loading' && <p role='status'>{t('Loading...')}</p>}
            {profile && (
              <p className='text-muted-foreground text-sm break-all'>
                {t('Current value')}:{' '}
                <output>{JSON.stringify(profile.display_name)}</output>
              </p>
            )}
            <div className='space-y-2'>
              <Label htmlFor={fieldId}>{t('Display name')}</Label>
              <Input
                id={fieldId}
                value={draft}
                onChange={(event) => setDraft(event.currentTarget.value)}
                disabled={phase === 'saving' || phase === 'loading'}
                aria-invalid={invalid}
                aria-describedby={`${fieldId}-limit`}
              />
              <p
                id={`${fieldId}-limit`}
                className='text-muted-foreground text-xs'
              >
                {t('Name must be between {{min}} and {{max}} characters', {
                  min: 0,
                  max: 20,
                })}{' '}
                · {[...draft].length}/20
              </p>
              {hasNativeDisplayNameControls(draft) && (
                <p role='alert' className='text-destructive text-sm'>
                  {t('Display name cannot contain control characters')}
                </p>
              )}
            </div>
            {(notice !== 'none' || unresolvedWrite) && (
              <Alert role='alert'>
                <AlertDescription>
                  {unresolvedWrite || notice === 'unknown'
                    ? t(
                        'Save result is unknown. Check the original operation before continuing.'
                      )
                    : notice === 'conflict'
                      ? t(
                          'Settings or users changed on the server. Reload and review again.'
                        )
                      : notice === 'invalid'
                        ? t('Unable to save user profile')
                        : t('Failed to load profile')}
                </AlertDescription>
              </Alert>
            )}
            <DialogFooter>
              <Button
                type='button'
                variant='outline'
                disabled={phase === 'saving' || phase === 'loading'}
                onClick={() => {
                  preserveDraft.current = true
                  setRevision((value) => value + 1)
                }}
              >
                {t('Reload')}
              </Button>
              <Button type='button' variant='outline' onClick={clear}>
                {t('Cancel')}
              </Button>
              <Button type='submit' disabled={disabled}>
                {phase === 'saving' ? t('Saving...') : t('Save changes')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
