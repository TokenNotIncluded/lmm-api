/*
Copyright (C) 2026 LIghtJUNction
*/
import { useQuery } from '@tanstack/react-query'
import { Check, ImageIcon, RefreshCw, Sparkles } from 'lucide-react'
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { api } from '@/lib/api'
import { cn } from '@/lib/utils'

import { getAssistantStatus } from '../assistant/api'
import { getPricing } from '../pricing/api'
import type { PricingModel } from '../pricing/types'
import {
  getDrawingRequestErrorKind,
  getDrawingRequestStatus,
} from './error-state'

type ImageResult = {
  url?: string
  b64_json?: string
  revised_prompt?: string
}

type ImageResponse = {
  data?: ImageResult[]
  error?: { message?: string }
  message?: string
}

function isImageModel(model: PricingModel): boolean {
  return model.supported_endpoint_types?.includes('image-generation') === true
}

function imageSource(image: ImageResult): string | undefined {
  if (image.url?.trim()) return image.url.trim()
  if (image.b64_json?.trim()) return `data:image/png;base64,${image.b64_json}`
  return undefined
}

function modelSupportsGroup(model: PricingModel, group: string): boolean {
  return (
    model.enable_groups.includes('all') || model.enable_groups.includes(group)
  )
}

function DrawingQueryErrorAlert(props: {
  title: string
  error: unknown
  onRetry: () => void | Promise<unknown>
}) {
  const { t } = useTranslation()
  const kind = getDrawingRequestErrorKind(props.error)
  const status = getDrawingRequestStatus(props.error)
  let description: string
  switch (kind) {
    case 'unauthenticated':
      description = t('Session expired!')
      break
    case 'forbidden':
      description = t('No permission to perform this action')
      break
    case 'unavailable':
      description = t('Please try again later.')
      break
    case 'network':
      description = t('Network connection failed or server not responding')
      break
    default:
      description = t('Request failed')
  }

  return (
    <Alert variant={kind === 'forbidden' ? 'default' : 'destructive'}>
      <AlertTitle>{props.title}</AlertTitle>
      <AlertDescription>
        {description}
        {status !== null ? ` (HTTP ${status})` : ''}
      </AlertDescription>
      <AlertAction>
        <Button
          type='button'
          size='sm'
          variant='outline'
          onClick={props.onRetry}
        >
          {t('Retry')}
        </Button>
      </AlertAction>
    </Alert>
  )
}

export function Drawing() {
  const { t } = useTranslation()
  const [prompt, setPrompt] = useState('')
  const [group, setGroup] = useState('')
  const [model, setModel] = useState('')
  const [size, setSize] = useState('')
  const [quality, setQuality] = useState('')
  const [count, setCount] = useState('1')
  const [results, setResults] = useState<ImageResult[]>([])
  const [error, setError] = useState<string | null>(null)
  const [generating, setGenerating] = useState(false)

  const accessQuery = useQuery({
    queryKey: ['assistant-status'],
    queryFn: getAssistantStatus,
    staleTime: 30_000,
    retry: false,
  })
  const pricingQuery = useQuery({
    queryKey: ['drawing-pricing'],
    queryFn: getPricing,
    staleTime: 5 * 60_000,
    retry: false,
  })
  const groupsQuery = useQuery({
    queryKey: ['drawing-user-groups'],
    queryFn: async () => {
      const response = await api.get<{
        success: boolean
        data?: Record<string, { desc: string; ratio: number | string }>
        message?: string
      }>('/api/user/self/groups')
      return response.data
    },
    staleTime: 60_000,
    retry: false,
  })

  const imageModels = useMemo(
    () =>
      (pricingQuery.data?.data ?? [])
        .filter(isImageModel)
        .sort((left, right) => left.model_name.localeCompare(right.model_name)),
    [pricingQuery.data?.data]
  )
  const groups = useMemo(() => {
    const usable =
      groupsQuery.data?.data ?? pricingQuery.data?.usable_group ?? {}
    return Object.keys(usable)
      .filter((name) =>
        imageModels.some((item) => modelSupportsGroup(item, name))
      )
      .sort((left, right) => left.localeCompare(right))
  }, [groupsQuery.data?.data, imageModels, pricingQuery.data?.usable_group])
  let selectedGroup = groups[0] ?? ''
  if (groups.includes('image-2')) selectedGroup = 'image-2'
  if (groups.includes(group)) selectedGroup = group
  const modelsForGroup = imageModels.filter((item) =>
    modelSupportsGroup(item, selectedGroup)
  )
  let selectedModel = modelsForGroup[0]?.model_name ?? ''
  if (modelsForGroup.some((item) => item.model_name === 'image-2')) {
    selectedModel = 'image-2'
  }
  if (modelsForGroup.some((item) => item.model_name === model)) {
    selectedModel = model
  }
  const groupDescription =
    groupsQuery.data?.data?.[selectedGroup]?.desc ??
    pricingQuery.data?.usable_group?.[selectedGroup]?.desc
  const accessGranted = accessQuery.data?.developer_access_granted === true
  const hasPrompt = prompt.trim().length > 0
  const configurationReady = Boolean(selectedGroup && selectedModel)
  const workflowIndex = results.length > 0 ? 3 : generating ? 2 : hasPrompt ? 1 : 0
  const workflowSteps = [
    {
      label: t('Describe an image'),
      detail: t('Be specific about the subject, mood, and style.'),
    },
    {
      label: t('Generation setup'),
      detail: t('Choose a route and output settings.'),
    },
    {
      label: t('Generate image'),
      detail: t('Billing follows the selected group configuration.'),
    },
    {
      label: t('Preview'),
      detail: t(
        'Review the generated images here when the request finishes.'
      ),
    },
  ]

  const sizePresets = useMemo(() => {
    const defaults = [{ value: '', label: t('Default') }]
    if (selectedModel === 'dall-e-2' || selectedModel === 'dall-e') {
      return [
        ...defaults,
        ...['256x256', '512x512', '1024x1024'].map((value) => ({
          value,
          label: value,
        })),
      ]
    }
    if (selectedModel === 'dall-e-3') {
      return [
        ...defaults,
        ...['1024x1024', '1024x1792', '1792x1024'].map((value) => ({
          value,
          label: value,
        })),
      ]
    }
    return [
      ...defaults,
      ...['1024x1024', '1024x1536', '1536x1024'].map((value) => ({
        value,
        label: value,
      })),
    ]
  }, [selectedModel, t])

  const qualityPresets = useMemo(() => {
    const defaults = [{ value: '', label: t('Default') }]
    if (selectedModel === 'dall-e-3') {
      return [
        ...defaults,
        ...['standard', 'hd'].map((value) => ({ value, label: value })),
      ]
    }
    if (selectedModel === 'gpt-image-1') {
      return [
        ...defaults,
        ...['auto', 'low', 'medium', 'high'].map((value) => ({
          value,
          label: value,
        })),
      ]
    }
    return [
      ...defaults,
      ...['standard', 'hd'].map((value) => ({ value, label: value })),
    ]
  }, [selectedModel, t])

  useEffect(() => {
    setSize((current) =>
      sizePresets.some((option) => option.value === current) ? current : ''
    )
    setQuality((current) =>
      qualityPresets.some((option) => option.value === current) ? current : ''
    )
  }, [qualityPresets, sizePresets])

  const generate = async () => {
    const cleanPrompt = prompt.trim()
    if (generating || !cleanPrompt || !selectedGroup || !selectedModel) return
    setGenerating(true)
    setError(null)
    setResults([])
    try {
      const response = await api.post<ImageResponse>(
        `/pg/images/generations?group=${encodeURIComponent(selectedGroup)}`,
        {
          prompt: cleanPrompt,
          model: selectedModel,
          n: Number(count),
          ...(size.trim() ? { size: size.trim() } : {}),
          ...(quality.trim() ? { quality: quality.trim() } : {}),
        },
        { skipBusinessError: true, skipErrorHandler: true }
      )
      if (response.data.error || !Array.isArray(response.data.data)) {
        throw new Error(
          response.data.error?.message ||
            response.data.message ||
            t('Unable to generate the image')
        )
      }
      const usableResults = response.data.data.filter(
        (image) => imageSource(image) !== undefined
      )
      setResults(usableResults)
      if (usableResults.length === 0) {
        setError(t('No images were returned'))
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : t('Unable to generate the image')
      )
    } finally {
      setGenerating(false)
    }
  }

  let content: ReactNode
  if (
    accessQuery.isLoading ||
    pricingQuery.isLoading ||
    groupsQuery.isLoading
  ) {
    content = (
      <div className='grid gap-4 sm:grid-cols-2'>
        <Skeleton className='h-10 w-full' />
        <Skeleton className='h-10 w-full' />
        <Skeleton className='h-32 w-full sm:col-span-2' />
      </div>
    )
  } else if (accessQuery.isError) {
    content = (
      <DrawingQueryErrorAlert
        title={t('Failed to load')}
        error={accessQuery.error}
        onRetry={() => accessQuery.refetch()}
      />
    )
  } else if (!accessGranted) {
    content = (
      <Alert>
        <AlertTitle>{t('L1 access required')}</AlertTitle>
        <AlertDescription>
          {t(
            'The drawing workbench is available after developer access is approved.'
          )}
        </AlertDescription>
      </Alert>
    )
  } else if (pricingQuery.isError || groupsQuery.isError) {
    content = (
      <div className='grid gap-3'>
        {pricingQuery.isError ? (
          <DrawingQueryErrorAlert
            title={t('Failed to load playground models')}
            error={pricingQuery.error}
            onRetry={() => pricingQuery.refetch()}
          />
        ) : null}
        {groupsQuery.isError ? (
          <DrawingQueryErrorAlert
            title={t('Failed to load playground groups')}
            error={groupsQuery.error}
            onRetry={() => groupsQuery.refetch()}
          />
        ) : null}
      </div>
    )
  } else if (groups.length === 0) {
    content = (
      <Alert variant='destructive'>
        <AlertTitle>{t('Image catalog unavailable')}</AlertTitle>
        <AlertDescription>
          {t(
            'No image-capable model and routing group is currently available.'
          )}
        </AlertDescription>
        <AlertAction>
          <Button
            type='button'
            size='sm'
            variant='outline'
            onClick={() => {
              void Promise.all([pricingQuery.refetch(), groupsQuery.refetch()])
            }}
          >
            {t('Retry')}
          </Button>
        </AlertAction>
      </Alert>
    )
  } else {
    content = (
      <div className='grid gap-6 lg:grid-cols-[14rem_minmax(0,1fr)] lg:items-start lg:gap-8'>
        <aside
          className='border-border/70 bg-muted/10 rounded-2xl border p-4 lg:sticky lg:top-6'
          aria-label={t('Workflow')}
        >
          <div className='flex items-center justify-between gap-3'>
            <span className='text-sm font-medium'>{t('Workflow')}</span>
            <span className='text-muted-foreground text-xs tabular-nums'>
              {workflowIndex + 1}/4
            </span>
          </div>
          <ol className='mt-4 grid grid-cols-4 gap-2 lg:grid-cols-1 lg:gap-1'>
            {workflowSteps.map((step, index) => {
              const complete = index < workflowIndex
              const current = index === workflowIndex
              return (
                <li
                  key={step.label}
                  aria-current={current ? 'step' : undefined}
                  className={cn(
                    'flex min-w-0 items-center gap-3 rounded-lg p-2 transition-colors lg:items-start',
                    current && 'bg-primary/10 text-foreground',
                    complete && !current && 'text-muted-foreground',
                    !current && !complete && 'text-muted-foreground/60'
                  )}
                >
                  <span
                    className={cn(
                      'border-border/80 flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-medium',
                      current && 'border-primary bg-primary text-primary-foreground',
                      complete && 'border-primary/30 bg-primary/10 text-primary'
                    )}
                  >
                    {complete ? (
                      <Check className='size-3.5' aria-hidden='true' />
                    ) : (
                      index + 1
                    )}
                  </span>
                  <span className='hidden min-w-0 lg:block'>
                    <span className='block truncate text-sm font-medium'>
                      {step.label}
                    </span>
                    <span className='text-muted-foreground mt-0.5 block text-xs leading-4'>
                      {step.detail}
                    </span>
                  </span>
                </li>
              )
            })}
          </ol>
        </aside>

        <div className='grid min-w-0 gap-6'>
          <section
            className='border-border/70 bg-card/30 rounded-2xl border p-5 sm:p-6'
            aria-labelledby='drawing-prompt'
          >
            <div className='mb-5 flex items-start justify-between gap-4'>
              <div>
                <div className='text-muted-foreground mb-2 flex items-center gap-2 text-xs font-medium tracking-[0.14em] uppercase'>
                  <Sparkles className='size-3.5' aria-hidden='true' />
                  {t('Describe an image')}
                </div>
                <h2 id='drawing-prompt' className='text-lg font-medium'>
                  {t('Prompt')}
                </h2>
              </div>
              <span className='text-muted-foreground text-xs tabular-nums'>
                {prompt.length}/2000
              </span>
            </div>
            <Textarea
              id='drawing-prompt-input'
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder={t('Describe what you want to see...')}
              maxLength={2000}
              rows={8}
              className='min-h-48 resize-y'
              aria-labelledby='drawing-prompt'
            />
            <p className='text-muted-foreground mt-3 text-xs leading-5'>
              {t('Be specific about the subject, mood, and style.')}
            </p>
          </section>

          <section
            className='border-border/70 bg-card/30 rounded-2xl border p-5 sm:p-6'
            aria-labelledby='drawing-setup'
          >
            <div className='mb-5 flex items-start justify-between gap-4'>
              <div>
                <p className='text-muted-foreground mb-2 text-xs font-medium tracking-[0.14em] uppercase'>
                  {t('Generation setup')}
                </p>
                <h2 id='drawing-setup' className='text-lg font-medium'>
                  {t('Configure')}
                </h2>
              </div>
              {configurationReady ? (
                <span className='text-primary text-xs font-medium'>
                  {t('Ready')}
                </span>
              ) : null}
            </div>
            <div className='grid gap-5 sm:grid-cols-2'>
              <div className='grid gap-2'>
                <Label htmlFor='drawing-group'>{t('Routing group')}</Label>
                <NativeSelect
                  id='drawing-group'
                  value={selectedGroup}
                  className='w-full'
                  onChange={(event) => setGroup(event.target.value)}
                >
                  {groups.map((item) => (
                    <NativeSelectOption key={item} value={item}>
                      {item}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                {groupDescription ? (
                  <p className='text-muted-foreground text-xs leading-relaxed break-words'>
                    {groupDescription}
                  </p>
                ) : null}
              </div>
              <div className='grid gap-2'>
                <Label htmlFor='drawing-model'>{t('Image model')}</Label>
                <NativeSelect
                  id='drawing-model'
                  value={selectedModel}
                  className='w-full'
                  onChange={(event) => setModel(event.target.value)}
                >
                  {modelsForGroup.map((item) => (
                    <NativeSelectOption
                      key={item.model_name}
                      value={item.model_name}
                    >
                      {item.model_name}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </div>
              <div className='grid gap-4 sm:grid-cols-2 sm:col-span-2'>
                <div className='grid gap-2'>
                  <Label htmlFor='drawing-size'>{t('Size (optional)')}</Label>
                  <NativeSelect
                    id='drawing-size'
                    value={size}
                    className='w-full'
                    onChange={(event) => setSize(event.target.value)}
                  >
                    {sizePresets.map((option) => (
                      <NativeSelectOption
                        key={option.value || 'default'}
                        value={option.value}
                      >
                        {option.label}
                      </NativeSelectOption>
                    ))}
                  </NativeSelect>
                </div>
                <div className='grid gap-2'>
                  <Label htmlFor='drawing-quality'>
                    {t('Quality (optional)')}
                  </Label>
                  <NativeSelect
                    id='drawing-quality'
                    value={quality}
                    className='w-full'
                    onChange={(event) => setQuality(event.target.value)}
                  >
                    {qualityPresets.map((option) => (
                      <NativeSelectOption
                        key={option.value || 'default'}
                        value={option.value}
                      >
                        {option.label}
                      </NativeSelectOption>
                    ))}
                  </NativeSelect>
                </div>
              </div>
              <div className='grid gap-2 sm:max-w-40'>
                <Label htmlFor='drawing-count'>{t('Images')}</Label>
                <NativeSelect
                  id='drawing-count'
                  value={count}
                  className='w-full'
                  onChange={(event) => setCount(event.target.value)}
                >
                  {[1, 2, 3, 4].map((value) => (
                    <NativeSelectOption key={value} value={String(value)}>
                      {value}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </div>
              <p className='text-muted-foreground self-end text-xs leading-5 sm:col-span-2'>
                {t('Billing follows the selected group configuration.')}
              </p>
            </div>
            <div className='border-border/70 bg-muted/20 mt-6 flex flex-col gap-3 rounded-xl border p-4 sm:flex-row sm:items-center sm:justify-between'>
              <div className='min-w-0'>
                <p className='text-sm font-medium'>
                  {generating
                    ? t('Generation in progress...')
                    : t('Ready to generate')}
                </p>
                <p className='text-muted-foreground mt-1 text-xs leading-5'>
                  {hasPrompt
                    ? t('Your request is ready to run.')
                    : t('Complete the brief and choose a model to continue.')}
                </p>
              </div>
              <Button
                type='button'
                className='h-11 w-full shrink-0 sm:w-auto sm:min-w-48'
                onClick={() => void generate()}
                disabled={
                  generating || !prompt.trim() || !selectedGroup || !selectedModel
                }
              >
                {generating ? (
                  <RefreshCw
                    className='mr-2 size-4 animate-spin'
                    aria-hidden='true'
                  />
                ) : (
                  <ImageIcon className='mr-2 size-4' aria-hidden='true' />
                )}
                {generating ? t('Generating...') : t('Generate image')}
              </Button>
            </div>
          </section>

          <section
            className='border-border/70 bg-card/30 min-w-0 rounded-2xl border p-5 sm:p-6'
            aria-live='polite'
            aria-labelledby='drawing-output'
          >
            <div className='mb-5 flex items-baseline justify-between gap-4'>
              <div>
                <p className='text-muted-foreground mb-2 text-xs font-medium tracking-[0.14em] uppercase'>
                  {t('Output review')}
                </p>
                <h2 id='drawing-output' className='text-lg font-medium'>
                  {t('Generated images')}
                </h2>
              </div>
              {results.length > 0 ? (
                <span className='text-muted-foreground text-xs'>
                  {results.length} · {selectedModel}
                </span>
              ) : null}
            </div>
            {error ? (
              <Alert variant='destructive' className='mb-5'>
                <AlertTitle>{t('Request failed')}</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
                <AlertAction>
                  <Button
                    type='button'
                    size='sm'
                    variant='outline'
                    onClick={() => void generate()}
                    disabled={generating}
                  >
                    {t('Retry')}
                  </Button>
                </AlertAction>
              </Alert>
            ) : null}
            {generating ? (
              <div className='bg-muted/5 text-muted-foreground flex min-h-64 flex-col items-center justify-center border border-dashed px-8 text-center sm:min-h-80'>
                <RefreshCw
                  className='text-primary mb-4 size-8 animate-spin'
                  aria-hidden='true'
                />
                <p className='text-sm'>{t('Generation in progress...')}</p>
                <p className='mt-2 max-w-sm text-xs leading-5'>
                  {t('Your request is ready to run.')}
                </p>
              </div>
            ) : results.length > 0 ? (
              <div className='grid gap-6 sm:grid-cols-2'>
                {results.map((image) => {
                  const src = imageSource(image)
                  if (!src) return null
                  return (
                    <figure
                      className='min-w-0'
                      key={image.url ?? image.b64_json ?? image.revised_prompt}
                    >
                      <img
                        src={src}
                        alt={image.revised_prompt || prompt}
                        className='h-auto max-h-[38rem] w-full rounded-lg object-contain'
                        loading='lazy'
                      />
                      {image.revised_prompt ? (
                        <figcaption className='text-muted-foreground mt-2 text-xs leading-5'>
                          {image.revised_prompt}
                        </figcaption>
                      ) : null}
                    </figure>
                  )
                })}
              </div>
            ) : (
              <div className='bg-muted/5 text-muted-foreground flex min-h-64 flex-col items-center justify-center border border-dashed px-8 text-center sm:min-h-80'>
                <ImageIcon
                  className='mb-4 size-8 opacity-50'
                  aria-hidden='true'
                />
                <p className='text-sm'>
                  {t('Your generated images will appear here.')}
                </p>
                <p className='mt-2 max-w-sm text-xs leading-5'>
                  {hasPrompt
                    ? t(
                        'Review the generated images here when the request finishes.'
                      )
                    : t('Complete the brief and choose a model to continue.')}
                </p>
              </div>
            )}
          </section>
        </div>
      </div>
    )
  }

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Drawing studio')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        <div className='mx-auto w-full max-w-6xl pb-16'>
          <header className='mb-10 grid gap-2'>
            <p className='text-muted-foreground text-sm'>
              {t(
                'Create images through the same safe, group-aware relay used by the API.'
              )}
            </p>
          </header>
          {content}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
