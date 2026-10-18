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
import { useQuery } from '@tanstack/react-query'
import { Pencil } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { CreditAmountInput } from '@/components/credit-amount-input'
import {
  SideDrawerSection,
  sideDrawerContentClassName,
  sideDrawerFooterClassName,
  sideDrawerFormClassName,
  sideDrawerHeaderClassName,
} from '@/components/drawer-layout'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Textarea } from '@/components/ui/textarea'
import { UserSourceDetails } from '@/features/acquisition/user-sources'
import { useMediaQuery } from '@/hooks'
import { useCreditInputDisplay } from '@/hooks/use-credit-input-display'
import {
  ADMIN_PERMISSION_ACTIONS,
  ADMIN_PERMISSION_RESOURCES,
  hasPermission,
  normalizeAdminPermissions,
} from '@/lib/admin-permissions'
import { formatQuota } from '@/lib/format'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  createUser,
  updateUser,
  getUser,
  getGroups,
  getPermissionCatalog,
} from '../api'
import { BINDING_FIELDS, ERROR_MESSAGES, SUCCESS_MESSAGES } from '../constants'
import {
  userFormSchema,
  type UserFormValues,
  USER_FORM_DEFAULT_VALUES,
  transformFormDataToPayload,
  transformUserToFormDefaults,
} from '../lib'
import { userSchema, type User } from '../types'
import { AssistantUserProfileEditor } from './assistant-user-profile-editor'
import { UserAnnouncementStatus } from './user-announcement-status'
import { UserQuotaDialog } from './user-quota-dialog'
import { useUsers } from './users-provider'

type UsersMutateDrawerProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  currentRow?: User
}

export function UsersMutateDrawer(props: UsersMutateDrawerProps) {
  const user = useAuthStore((state) => state.auth.user)
  const sid = useAuthStore((state) => state.auth.session?.sid)
  const key = `${user?.id}:${user?.role}:${sid}:${props.currentRow?.id ?? 'create'}:${props.open}`
  // A new actor, target or opening gets an entirely new form, before paint.
  return <UsersMutateDrawerForm key={key} {...props} />
}

function UsersMutateDrawerForm({
  open,
  onOpenChange,
  currentRow,
}: UsersMutateDrawerProps) {
  const { t } = useTranslation()
  const isMobile = useMediaQuery('(max-width: 639px)')
  const isUpdate = !!currentRow
  const { triggerRefresh } = useUsers()
  const currentUser = useAuthStore((s) => s.auth.user)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [quotaDialogOpen, setQuotaDialogOpen] = useState(false)
  const [loadedTarget, setLoadedTarget] = useState<User>()
  const [targetError, setTargetError] = useState(false)
  const [targetReload, setTargetReload] = useState(0)
  const mounted = useRef(true)
  const targetId = currentRow?.id
  const actorId = currentUser?.id
  const actorRole = currentUser?.role
  const actorMatches = () => {
    const auth = useAuthStore.getState().auth
    return (
      mounted.current &&
      auth.user?.id === actorId &&
      auth.user?.role === actorRole &&
      auth.session?.sid === sessionId
    )
  }
  useLayoutEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  // Fetch groups
  const { data: groupsData } = useQuery({
    queryKey: ['groups'],
    queryFn: getGroups,
    enabled: open,
    staleTime: 5 * 60 * 1000,
  })

  const groups = groupsData?.data || []

  // Recheck the server-owned catalog for this actor whenever the drawer opens.
  const permissionCatalogQuery = useQuery({
    queryKey: [
      'admin-permission-catalog',
      currentUser?.id,
      currentUser?.role,
      sessionId,
    ],
    queryFn: async ({ signal }) => {
      if (!actorMatches()) throw new Error('Session changed')
      const catalog = await getPermissionCatalog({
        authScope: { userId: actorId, sessionId },
        signal,
        skipAuthRefresh: true,
        skipBusinessError: true,
        skipErrorHandler: true,
      })
      if (!actorMatches()) throw new Error('Session changed')
      return catalog
    },
    enabled: open && currentUser?.role === ROLE.SUPER_ADMIN,
    staleTime: 0,
    retry: false,
  })
  const permissionCatalog = permissionCatalogQuery.data

  const form = useForm<UserFormValues>({
    resolver: zodResolver(userFormSchema),
    defaultValues: USER_FORM_DEFAULT_VALUES,
  })

  // Only the current actor's successful read of this exact target can unlock writes.
  useEffect(() => {
    const controller = new AbortController()
    let active = true
    form.reset(USER_FORM_DEFAULT_VALUES)
    setLoadedTarget(undefined)
    setTargetError(false)
    if (open && targetId !== undefined && actorId !== undefined) {
      void (async () => {
        try {
          const result = await getUser(targetId, {
            authScope: { userId: actorId, sessionId },
            signal: controller.signal,
            skipAuthRefresh: true,
            skipBusinessError: true,
            skipErrorHandler: true,
            timeout: 10_000,
          })
          const parsed = userSchema.safeParse(result.data)
          if (
            !result.success ||
            !parsed.success ||
            parsed.data.id !== targetId
          ) {
            throw new Error('User profile unavailable')
          }
          const defaults = transformUserToFormDefaults(parsed.data)
          const auth = useAuthStore.getState().auth
          if (
            !active ||
            controller.signal.aborted ||
            auth.user?.id !== actorId ||
            auth.user?.role !== actorRole ||
            auth.session?.sid !== sessionId
          ) {
            return
          }
          form.reset(defaults)
          setLoadedTarget(parsed.data)
        } catch {
          if (active && !controller.signal.aborted) setTargetError(true)
        }
      })()
    }
    return () => {
      active = false
      controller.abort()
    }
  }, [open, targetId, actorId, actorRole, sessionId, form, targetReload])

  const { label: currencyLabel } = useCreditInputDisplay()

  const currentQuotaRaw = form.watch('quota_credits') || 0
  const selectedRole = form.watch('role')
  const canEditAdminPermissions = currentUser?.role === ROLE.SUPER_ADMIN
  const targetIsAdmin = (selectedRole ?? currentRow?.role ?? 0) >= ROLE.ADMIN
  const targetPermissionsUnavailable =
    isUpdate &&
    canEditAdminPermissions &&
    (loadedTarget?.role ?? 0) >= ROLE.ADMIN &&
    (!loadedTarget?.admin_permissions ||
      (permissionCatalog &&
        permissionCatalog.resources.some((resource) =>
          resource.actions.some(
            (action) =>
              typeof loadedTarget?.admin_permissions?.[resource.resource]?.[
                action.action
              ] !== 'boolean'
          )
        )))
  const targetUnavailable =
    !open ||
    !currentUser ||
    (isUpdate &&
      (!loadedTarget ||
        loadedTarget.id !== targetId ||
        targetError ||
        targetPermissionsUnavailable))
  const permissionsUnavailable =
    canEditAdminPermissions &&
    targetIsAdmin &&
    (!permissionCatalog ||
      permissionCatalogQuery.isError ||
      permissionCatalogQuery.isFetching)

  const onSubmit = async (data: UserFormValues) => {
    if (
      !actorMatches() ||
      targetUnavailable ||
      permissionsUnavailable ||
      isSubmitting
    ) {
      return
    }
    if (!isUpdate) {
      const passwordLength = data.password?.length || 0
      if (passwordLength < 8 || passwordLength > 20) {
        form.setError('password', {
          type: 'manual',
          message: t('Password must be between 8 and 20 characters'),
        })
        return
      }
    }

    setIsSubmitting(true)
    try {
      const payload = transformFormDataToPayload(
        data,
        currentRow?.id,
        canEditAdminPermissions && targetIsAdmin ? permissionCatalog : undefined
      )
      const result = currentRow
        ? await updateUser(
            {
              ...payload,
              id: currentRow.id,
            },
            { authScope: { userId: actorId, sessionId }, skipAuthRefresh: true }
          )
        : await createUser(payload, {
            authScope: { userId: actorId, sessionId },
            skipAuthRefresh: true,
          })

      if (!actorMatches()) return

      if (result.success) {
        toast.success(
          isUpdate
            ? t(SUCCESS_MESSAGES.USER_UPDATED)
            : t(SUCCESS_MESSAGES.USER_CREATED)
        )
        onOpenChange(false)
        triggerRefresh()
      } else {
        toast.error(
          result.message ||
            (isUpdate
              ? t(ERROR_MESSAGES.UPDATE_FAILED)
              : t(ERROR_MESSAGES.CREATE_FAILED))
        )
      }
    } catch {
      if (actorMatches()) toast.error(t(ERROR_MESSAGES.UNEXPECTED))
    } finally {
      setIsSubmitting(false)
    }
  }

  const refreshUserData = () => {
    if (!currentRow) return
    setTargetReload((value) => value + 1)
    triggerRefresh()
  }

  const userDetails = isUpdate && currentRow && open && (
    <>
      <UserAnnouncementStatus userID={currentRow.id} />
      <UserSourceDetails userID={currentRow.id} />
    </>
  )

  return (
    <>
      <Sheet
        open={open}
        onOpenChange={(v) => {
          onOpenChange(v)
          if (!v) {
            form.reset()
          }
        }}
      >
        <SheetContent
          className={sideDrawerContentClassName(
            'sm:max-w-[600px] max-sm:[&>button[data-slot=sheet-close]]:size-11'
          )}
        >
          <SheetHeader
            className={sideDrawerHeaderClassName(
              'shrink-0 pr-16 pt-[max(0.75rem,env(safe-area-inset-top))] sm:pr-6 sm:pt-4'
            )}
          >
            <SheetTitle>
              {isUpdate ? t('Update') : t('Create')} {t('User')}
            </SheetTitle>
            <SheetDescription>
              {isUpdate
                ? t('Update the user by providing necessary info.')
                : t('Add a new user by providing necessary info.')}
            </SheetDescription>
          </SheetHeader>
          <Form {...form}>
            <form
              id='user-form'
              onSubmit={form.handleSubmit(onSubmit)}
              className={sideDrawerFormClassName(
                'max-sm:[&_input]:min-h-11 max-sm:[&_[data-slot=select-trigger]]:min-h-11 max-sm:[&_[data-slot=select-trigger]]:w-full max-sm:[&_button:not([role=checkbox]):not([role=switch])]:min-h-11 max-sm:[&_label[for=assistant-profile-enabled]]:min-h-11 max-sm:[&_label[for=assistant-profile-enabled]]:min-w-11'
              )}
            >
              {isUpdate && (targetError || targetPermissionsUnavailable) ? (
                <div role='alert' className='space-y-2'>
                  <p>{t('Failed to load profile')}</p>
                  <Button
                    type='button'
                    variant='outline'
                    onClick={() => setTargetReload((value) => value + 1)}
                  >
                    {t('Retry')}
                  </Button>
                </div>
              ) : isUpdate && !loadedTarget ? (
                <p role='status'>{t('Loading...')}</p>
              ) : null}
              <fieldset disabled={!!targetUnavailable} className='contents'>
                {/* Basic Information */}
                {!isMobile && userDetails}
                <SideDrawerSection>
                  <h3 className='text-sm font-medium'>
                    {t('Basic Information')}
                  </h3>

                  <FormField
                    control={form.control}
                    name='username'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('Username')}</FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            placeholder={t('Enter username')}
                            disabled={isUpdate}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  {!isUpdate && (
                    <FormField
                      control={form.control}
                      name='role'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t('Role')}</FormLabel>
                          <Select
                            items={[
                              { value: '1', label: t('Common User') },
                              { value: '10', label: t('Admin') },
                            ]}
                            onValueChange={(value) =>
                              value !== null &&
                              field.onChange(Number.parseInt(value))
                            }
                            value={String(field.value)}
                          >
                            <FormControl>
                              <SelectTrigger>
                                <SelectValue placeholder={t('Select a role')} />
                              </SelectTrigger>
                            </FormControl>
                            <SelectContent alignItemWithTrigger={false}>
                              <SelectGroup>
                                <SelectItem value='1'>
                                  {t('Common User')}
                                </SelectItem>
                                <SelectItem value='10'>{t('Admin')}</SelectItem>
                              </SelectGroup>
                            </SelectContent>
                          </Select>
                          <FormDescription>
                            {t("Set the user's role (cannot be Root)")}
                          </FormDescription>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  )}

                  <FormField
                    control={form.control}
                    name='display_name'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('Display Name')}</FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            placeholder={t('Enter display name')}
                          />
                        </FormControl>
                        <FormDescription>
                          {t('Leave empty to use username')}
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name='password'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('Password')}</FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            type='password'
                            placeholder={
                              isUpdate
                                ? t('Leave empty to keep unchanged')
                                : t('Enter password (8-20 characters)')
                            }
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </SideDrawerSection>

                {isMobile && userDetails}

                {/* Group & Quota Settings (Update only) */}
                {isUpdate && (
                  <SideDrawerSection>
                    <h3 className='text-sm font-medium'>
                      {t('Group & Quota')}
                    </h3>

                    <FormField
                      control={form.control}
                      name='group'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t('Group')}</FormLabel>
                          <Select
                            items={groups.map((group) => ({
                              value: group,
                              label: group,
                            }))}
                            onValueChange={field.onChange}
                            value={field.value}
                          >
                            <FormControl>
                              <SelectTrigger>
                                <SelectValue
                                  placeholder={t('Select a group')}
                                />
                              </SelectTrigger>
                            </FormControl>
                            <SelectContent alignItemWithTrigger={false}>
                              <SelectGroup>
                                {groups.map((group) => (
                                  <SelectItem key={group} value={group}>
                                    {group}
                                  </SelectItem>
                                ))}
                              </SelectGroup>
                            </SelectContent>
                          </Select>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name='quota_credits'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>
                            {t('Remaining Quota ({{currency}})', {
                              currency: currencyLabel,
                            })}
                          </FormLabel>
                          <div className='flex flex-col gap-2 sm:flex-row'>
                            <FormControl>
                              <CreditAmountInput
                                value={field.value}
                                onValueChange={field.onChange}
                                allowNegative
                                readOnly
                                className='min-w-0 flex-1 tabular-nums'
                              />
                            </FormControl>
                            <Button
                              type='button'
                              variant='outline'
                              onClick={() => setQuotaDialogOpen(true)}
                            >
                              <Pencil className='mr-1 h-4 w-4' />
                              {t('Adjust Quota')}
                            </Button>
                          </div>
                          <FormDescription>
                            {formatQuota(field.value || 0)}
                          </FormDescription>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name='remark'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t('Remark')}</FormLabel>
                          <FormControl>
                            <Textarea
                              {...field}
                              placeholder={t(
                                'Admin notes (only visible to admins)'
                              )}
                              rows={3}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </SideDrawerSection>
                )}

                {canEditAdminPermissions && targetIsAdmin && (
                  <SideDrawerSection>
                    <h3 className='text-sm font-medium'>
                      {t('Admin Permissions')}
                    </h3>
                    <p className='text-muted-foreground text-xs'>
                      {t(
                        'Default administrator permissions can be overridden for this user.'
                      )}
                    </p>
                    {permissionCatalogQuery.isError ? (
                      <div role='alert' className='space-y-2'>
                        <p className='text-destructive text-sm'>
                          {t(
                            'Administrator permissions could not be loaded. Retry before saving.'
                          )}
                        </p>
                        <Button
                          type='button'
                          variant='outline'
                          disabled={permissionCatalogQuery.isFetching}
                          onClick={() => void permissionCatalogQuery.refetch()}
                        >
                          {t('Retry')}
                        </Button>
                      </div>
                    ) : permissionsUnavailable ? (
                      <p
                        role='status'
                        className='text-muted-foreground text-sm'
                      >
                        {t('Loading...')}
                      </p>
                    ) : permissionCatalog ? (
                      <FormField
                        control={form.control}
                        name='admin_permissions'
                        render={({ field }) => {
                          const selected = normalizeAdminPermissions(
                            field.value,
                            permissionCatalog
                          )
                          return (
                            <FormItem>
                              <div className='space-y-3'>
                                {permissionCatalog.resources.map((resource) => (
                                  <div
                                    key={resource.resource}
                                    className='space-y-2 rounded-md border p-3'
                                  >
                                    <div className='text-sm font-medium'>
                                      {t(resource.label_key)}
                                    </div>
                                    <div className='space-y-2'>
                                      {resource.actions.map((option) => (
                                        <label
                                          key={option.action}
                                          className='flex min-h-11 items-start gap-3 py-2 sm:min-h-0 sm:py-0'
                                        >
                                          <Checkbox
                                            checked={
                                              selected[resource.resource]?.[
                                                option.action
                                              ] === true
                                            }
                                            onCheckedChange={(checked) => {
                                              field.onChange({
                                                ...selected,
                                                [resource.resource]: {
                                                  ...selected[
                                                    resource.resource
                                                  ],
                                                  [option.action]:
                                                    checked === true,
                                                },
                                              })
                                            }}
                                          />
                                          <span className='flex flex-col gap-1'>
                                            <span className='text-sm font-medium'>
                                              {t(option.label_key)}
                                            </span>
                                            <span className='text-muted-foreground text-xs'>
                                              {t(option.description_key)}
                                            </span>
                                          </span>
                                        </label>
                                      ))}
                                    </div>
                                  </div>
                                ))}
                              </div>
                              <FormMessage />
                            </FormItem>
                          )
                        }}
                      />
                    ) : null}
                    {currentUser && (
                      <p className='text-muted-foreground text-xs'>
                        {hasPermission(
                          currentUser,
                          ADMIN_PERMISSION_RESOURCES.CHANNEL,
                          ADMIN_PERMISSION_ACTIONS.SENSITIVE_WRITE
                        )
                          ? t(
                              'Your account can edit sensitive channel settings.'
                            )
                          : t(
                              'Your account cannot edit sensitive channel settings.'
                            )}
                      </p>
                    )}
                  </SideDrawerSection>
                )}

                {isUpdate && currentRow && (
                  <AssistantUserProfileEditor
                    userId={currentRow.id}
                    open={open}
                  />
                )}

                {/* Binding Information (Read-only) */}
                {isUpdate && (
                  <SideDrawerSection>
                    <h3 className='text-sm font-medium'>
                      {t('Binding Information')}
                    </h3>
                    <p className='text-muted-foreground text-xs'>
                      {t(
                        'Third-party account bindings (read-only, managed by user in profile settings)'
                      )}
                    </p>

                    <div className='flex flex-col gap-3'>
                      {BINDING_FIELDS.map(({ key, label }) => (
                        <div key={key}>
                          <Label className='text-muted-foreground text-xs'>
                            {t(label)}
                          </Label>
                          <Input
                            value={
                              (currentRow?.[key as keyof User] as string) || '-'
                            }
                            disabled
                            className='mt-1'
                          />
                        </div>
                      ))}
                    </div>
                  </SideDrawerSection>
                )}
              </fieldset>
            </form>
          </Form>
          <SheetFooter
            className={sideDrawerFooterClassName(
              'shrink-0 pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:pb-4'
            )}
          >
            <SheetClose
              render={
                <Button variant='outline' className='min-h-11 sm:min-h-9' />
              }
            >
              {t('Close')}
            </SheetClose>
            <Button
              form='user-form'
              type='submit'
              disabled={
                isSubmitting || permissionsUnavailable || !!targetUnavailable
              }
              className='min-h-11 sm:min-h-9'
            >
              {isSubmitting ? t('Saving...') : t('Save changes')}
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>

      {/* Adjust Quota Dialog */}
      {currentRow && (
        <UserQuotaDialog
          open={quotaDialogOpen}
          onOpenChange={setQuotaDialogOpen}
          userId={currentRow.id}
          currentQuota={currentQuotaRaw || 0}
          onSuccess={refreshUserData}
        />
      )}
    </>
  )
}
