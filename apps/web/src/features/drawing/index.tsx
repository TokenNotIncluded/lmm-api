/*
Copyright (C) 2026 LIghtJUNction
*/
import { useQuery } from '@tanstack/react-query'
import { ImageIcon, RefreshCw, Sparkles } from 'lucide-react'
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
      setResults(response.data.data)
      if (response.data.data.length === 0) {
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
      <div className='grid gap-12 lg:grid-cols-[minmax(20rem,0.85fr)_minmax(0,1.15fr)] lg:gap-16'>
        <section
          className='grid content-start gap-7'
          aria-labelledby='drawing-prompt'
        >
          <div className='grid gap-3'>
            <div className='flex items-center gap-2'>
              <Sparkles
                className='text-muted-foreground size-4'
                aria-hidden='true'
              />
              <Label htmlFor='drawing-prompt'>{t('Describe an image')}</Label>
            </div>
            <Textarea
              id='drawing-prompt'
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder={t('Describe what you want to see...')}
              maxLength={2000}
              rows={10}
              className='min-h-56 resize-y'
            />
            <div className='text-muted-foreground flex justify-between text-xs'>
              <span>
                {t('Be specific about the subject, mood, and style.')}
              </span>
              <span>{prompt.length}/2000</span>
            </div>
          </div>
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='grid gap-2'>
              <Label htmlFor='drawing-group'>{t('Routing group')}</Label>
              <NativeSelect
                id='drawing-group'
                value={selectedGroup}
                onChange={(event) => setGroup(event.target.value)}
              >
                {groups.map((item) => (
                  <NativeSelectOption key={item} value={item}>
                    {item}
                    {item === selectedGroup && groupDescription
                      ? ` · ${groupDescription}`
                      : ''}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </div>
            <div className='grid gap-2'>
              <Label htmlFor='drawing-model'>{t('Image model')}</Label>
              <NativeSelect
                id='drawing-model'
                value={selectedModel}
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
            <p className='text-muted-foreground text-xs sm:col-span-2'>
              {t('Billing follows the selected group configuration.')}
            </p>
          </div>
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='grid gap-2'>
              <Label htmlFor='drawing-size'>{t('Size (optional)')}</Label>
              <NativeSelect
                id='drawing-size'
                value={size}
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
              <Label htmlFor='drawing-quality'>{t('Quality (optional)')}</Label>
              <NativeSelect
                id='drawing-quality'
                value={quality}
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
              onChange={(event) => setCount(event.target.value)}
            >
              {[1, 2, 3, 4].map((value) => (
                <NativeSelectOption key={value} value={String(value)}>
                  {value}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className='flex flex-col gap-3 sm:flex-row sm:items-center'>
            <Button
              type='button'
              className='h-11 w-full sm:w-auto sm:min-w-48'
              onClick={() => void generate()}
              disabled={generating || !prompt.trim() || !selectedModel}
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
            {error ? <p className='text-destructive text-sm'>{error}</p> : null}
          </div>
        </section>
        <section className='min-w-0 lg:border-l lg:pl-12' aria-live='polite'>
          <div className='mb-5 flex items-baseline justify-between gap-4'>
            <div>
              <p className='text-muted-foreground text-xs tracking-[0.16em] uppercase'>
                {t('Preview')}
              </p>
              <h2 className='mt-1 text-lg'>{t('Generated images')}</h2>
            </div>
            {results.length > 0 ? (
              <span className='text-muted-foreground text-xs'>
                {results.length} · {selectedModel}
              </span>
            ) : null}
          </div>
          {results.length > 0 ? (
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
            <div className='bg-muted/5 text-muted-foreground flex min-h-56 flex-col items-center justify-center border border-dashed px-8 text-center sm:min-h-72 lg:min-h-[30rem]'>
              <ImageIcon
                className='mb-4 size-8 opacity-50'
                aria-hidden='true'
              />
              <p className='text-sm'>
                {t('Your generated images will appear here.')}
              </p>
              <p className='mt-2 max-w-sm text-xs leading-5'>
                {t(
                  'Describe an image, choose a group, and generate a preview.'
                )}
              </p>
            </div>
          )}
        </section>
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
