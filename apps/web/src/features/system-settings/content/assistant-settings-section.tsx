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
import { useEffect } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

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
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
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
import {
  ASSISTANT_SEARCH_PROVIDERS,
  type AssistantSearchProvider,
} from '../types'
import { safeNumberFieldProps } from '../utils/numeric-field'
import {
  assistantSettingsSchema,
  type AssistantSettingsFormValues,
} from './assistant-settings-schema'

export function AssistantSettingsSection(props: {
  defaultValues: AssistantSettingsFormValues
}) {
  const { t } = useTranslation()
  const updateOption = useUpdateOption()
  const form = useForm<AssistantSettingsFormValues>({
    resolver: zodResolver(assistantSettingsSchema),
    defaultValues: props.defaultValues,
  })

  useEffect(() => {
    form.reset(props.defaultValues)
  }, [form, props.defaultValues])

  const onSubmit = async (values: AssistantSettingsFormValues) => {
    const updates = Object.entries(values).filter(
      ([key, value]) =>
        value !== props.defaultValues[key as keyof AssistantSettingsFormValues]
    )

    for (const [key, value] of updates) {
      await updateOption.mutateAsync({ key, value })
    }
  }

  const enabled = form.watch('AssistantEnabled')
  const agentLoopEnabled = form.watch('AssistantAgentLoopEnabled')
  const cacheEnabled = form.watch('AssistantCacheEnabled')
  const retentionEnabled = form.watch('AssistantRetentionEnabled')
  const searchProvider = form.watch('AssistantSearchProvider')
  const searchProviderDescription: Record<AssistantSearchProvider, string> = {
    none: t('Disable assistant web search.'),
    exa: t('Uses the official Exa Search API from the server.'),
    tavily: t('Uses the official Tavily Search API from the server.'),
    brave: t('Uses the official Brave Search API from the server.'),
    generic_http: t(
      'Send a GET request with the q query parameter to a custom endpoint.'
    ),
    mcp_streamable_http: t(
      'Connect to an MCP server over Streamable HTTP from the server.'
    ),
  }

  return (
    <SettingsSection title={t('AI assistant settings')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={updateOption.isPending}
            saveLabel='Save assistant settings'
          />

          <FormField
            control={form.control}
            name='AssistantEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable AI assistant')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Show the assistant launcher and allow assistant conversations.'
                    )}
                  </FormDescription>
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                  />
                </FormControl>
                <FormMessage />
              </SettingsSwitchItem>
            )}
          />

          <div className='grid gap-6'>
            <FormField
              control={form.control}
              name='AssistantModel'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Default assistant model')}</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      disabled={!enabled}
                      placeholder='deepseek-v4-flash'
                      autoComplete='off'
                    />
                  </FormControl>
                  <FormDescription>
                    {t(
                      'Model ID used for assistant conversations. Token usage is charged to the enabled super administrator account.'
                    )}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>

          <div className='border-border/60 bg-muted/20 space-y-4 rounded-lg border p-4'>
            <div>
              <h3 className='text-sm font-medium'>{t('Assistant behavior')}</h3>
              <p className='text-muted-foreground mt-1 text-sm'>
                {t(
                  'Customize the assistant without changing its built-in privacy and confirmation rules.'
                )}
              </p>
            </div>

            <FormField
              control={form.control}
              name='AssistantPersona'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Personality')}</FormLabel>
                  <FormControl>
                    <Textarea
                      {...field}
                      disabled={!enabled}
                      rows={3}
                      maxLength={2000}
                      placeholder={t(
                        'Helpful onboarding coach, concise technical writer, and honest product guide.'
                      )}
                    />
                  </FormControl>
                  <FormDescription>
                    {t('Describe the tone, role, and communication style.')}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name='AssistantSystemPrompt'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Administrator instructions')}</FormLabel>
                  <FormControl>
                    <Textarea
                      {...field}
                      disabled={!enabled}
                      rows={5}
                      maxLength={8000}
                      placeholder={t(
                        'Add product policies, support workflow, and facts the assistant should follow.'
                      )}
                    />
                  </FormControl>
                  <FormDescription>
                    {t(
                      'These instructions are appended to the assistant context.'
                    )}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name='AssistantSearchProvider'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Search provider')}</FormLabel>
                  <Select
                    value={field.value}
                    onValueChange={(value) => {
                      if (
                        typeof value === 'string' &&
                        (
                          ASSISTANT_SEARCH_PROVIDERS as readonly string[]
                        ).includes(value)
                      ) {
                        field.onChange(value)
                      }
                    }}
                  >
                    <FormControl>
                      <SelectTrigger className='w-full' disabled={!enabled}>
                        <SelectValue
                          placeholder={t('Select a search provider')}
                        />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent alignItemWithTrigger={false}>
                      <SelectGroup>
                        <SelectItem value='none'>{t('Disabled')}</SelectItem>
                        <SelectItem value='exa'>Exa</SelectItem>
                        <SelectItem value='tavily'>Tavily</SelectItem>
                        <SelectItem value='brave'>Brave Search</SelectItem>
                        <SelectItem value='generic_http'>
                          {t('Custom HTTP')}
                        </SelectItem>
                        <SelectItem value='mcp_streamable_http'>
                          {t('MCP (Streamable HTTP)')}
                        </SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <FormDescription>
                    {searchProviderDescription[searchProvider]}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            {searchProvider === 'generic_http' && (
              <FormField
                control={form.control}
                name='AssistantSearchURL'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Search tool API URL')}</FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        disabled={!enabled}
                        placeholder='https://search.example/api/search'
                        autoComplete='off'
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'The assistant sends a GET request with the query parameter q.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}

            {searchProvider === 'mcp_streamable_http' && (
              <>
                <FormField
                  control={form.control}
                  name='AssistantSearchURL'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('MCP Streamable HTTP endpoint')}</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          disabled={!enabled}
                          placeholder='https://search.example/mcp'
                          autoComplete='off'
                        />
                      </FormControl>
                      <FormDescription>
                        {t(
                          'The endpoint and credentials are used only by the server.'
                        )}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='AssistantSearchMCPTool'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {t('Optional MCP search tool name')}
                      </FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          disabled={!enabled}
                          placeholder='web_search'
                          autoComplete='off'
                        />
                      </FormControl>
                      <FormDescription>
                        {t('Leave empty to automatically find a search tool.')}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            <FormField
              control={form.control}
              name='AssistantSearchAPIKey'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Search tool API key')}</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      type='password'
                      disabled={!enabled}
                      placeholder={t('Leave blank to keep the existing key')}
                      autoComplete='new-password'
                    />
                  </FormControl>
                  <FormDescription>
                    {t(
                      'The key is stored server-side and is never shown in the options response.'
                    )}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name='AssistantSkills'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Skills and playbooks')}</FormLabel>
                  <FormControl>
                    <Textarea
                      {...field}
                      disabled={!enabled}
                      rows={6}
                      maxLength={12000}
                      placeholder={t(
                        'One skill or workflow per line. Example: CC Switch troubleshooting: check endpoint, model ID, key, then run a small request.'
                      )}
                    />
                  </FormControl>
                  <FormDescription>
                    {t(
                      'Give the agent reusable guidance for platform setup and support workflows.'
                    )}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>

          <div className='border-border/60 bg-muted/20 space-y-4 rounded-lg border p-4'>
            <div>
              <h3 className='text-sm font-medium'>{t('Agent runtime')}</h3>
              <p className='text-muted-foreground mt-1 text-sm'>
                {t(
                  'Configure the assistant tool loop and its safety limits. Tool actions that change an account still require explicit confirmation.'
                )}
              </p>
            </div>

            <FormField
              control={form.control}
              name='AssistantAgentLoopEnabled'
              render={({ field }) => (
                <SettingsSwitchItem>
                  <SettingsSwitchContent>
                    <FormLabel>{t('Enable agent tool loop')}</FormLabel>
                    <FormDescription>
                      {t(
                        'Allow the assistant to call safe information and calculation tools before producing its final answer.'
                      )}
                    </FormDescription>
                  </SettingsSwitchContent>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                      disabled={!enabled}
                    />
                  </FormControl>
                  <FormMessage />
                </SettingsSwitchItem>
              )}
            />

            <div className='grid gap-6 sm:grid-cols-2'>
              <FormField
                control={form.control}
                name='AssistantMaxSteps'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Maximum agent steps')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={1}
                        max={12}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!enabled || !agentLoopEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'Maximum number of model/tool turns in one assistant request (1–12).'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='AssistantTimeoutSeconds'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Agent timeout (seconds)')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={5}
                        max={120}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!enabled || !agentLoopEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'Hard limit for the complete agent loop (5–120 seconds).'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>

            <FormField
              control={form.control}
              name='AssistantCacheEnabled'
              render={({ field }) => (
                <SettingsSwitchItem>
                  <SettingsSwitchContent>
                    <FormLabel>
                      {t('Cache identical first questions')}
                    </FormLabel>
                    <FormDescription>
                      {t(
                        'Return the same successful answer for an identical first question during the cache window without calling an upstream model.'
                      )}
                    </FormDescription>
                  </SettingsSwitchContent>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                      disabled={!enabled}
                    />
                  </FormControl>
                  <FormMessage />
                </SettingsSwitchItem>
              )}
            />

            <FormField
              control={form.control}
              name='AssistantCacheTTLMinutes'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Cache window (minutes)')}</FormLabel>
                  <FormControl>
                    <Input
                      type='number'
                      min={0}
                      max={10080}
                      step={1}
                      {...safeNumberFieldProps(field)}
                      disabled={!enabled || !cacheEnabled}
                    />
                  </FormControl>
                  <FormDescription>
                    {t('Set to 0 to disable caching; the maximum is 7 days.')}
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>

          <div className='grid gap-5 border-t pt-6'>
            <div>
              <h3 className='text-sm font-medium'>
                {t('Conversation retention')}
              </h3>
              <p className='text-muted-foreground mt-1 text-sm'>
                {t(
                  'Automatically remove old assistant conversations in small batches. Revealed or expired private-card secrets are erased separately.'
                )}
              </p>
            </div>

            <FormField
              control={form.control}
              name='AssistantRetentionEnabled'
              render={({ field }) => (
                <SettingsSwitchItem>
                  <SettingsSwitchContent>
                    <FormLabel>{t('Enable scheduled cleanup')}</FormLabel>
                    <FormDescription>
                      {t(
                        'Run conversation cleanup as a background system task.'
                      )}
                    </FormDescription>
                  </SettingsSwitchContent>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                  <FormMessage />
                </SettingsSwitchItem>
              )}
            />

            <div className='grid gap-5 sm:grid-cols-2'>
              <FormField
                control={form.control}
                name='AssistantActiveRetentionDays'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Active conversations (days)')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={7}
                        max={3650}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!retentionEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t('Keep inactive conversations for 7–3650 days.')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='AssistantArchivedRetentionDays'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Archived conversations (days)')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={1}
                        max={3650}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!retentionEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t('Keep archived conversations for 1–3650 days.')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='AssistantSecurityRetentionDays'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Security reports (days)')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={30}
                        max={3650}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!retentionEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'Keep terminated security conversations for 30–3650 days.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='AssistantRetentionIntervalHours'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Cleanup interval (hours)')}</FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={1}
                        max={168}
                        step={1}
                        {...safeNumberFieldProps(field)}
                        disabled={!retentionEnabled}
                      />
                    </FormControl>
                    <FormDescription>
                      {t('Run every 1–168 hours; each pass is memory-bounded.')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
          </div>
        </SettingsForm>
      </Form>
    </SettingsSection>
  )
}
