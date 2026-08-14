/*
Copyright (C) 2026 LIghtJUNction
*/
import { useQuery } from '@tanstack/react-query'
import { ImageIcon, RefreshCw } from 'lucide-react'
import { useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { api } from '@/lib/api'

import { getAssistantStatus } from '../assistant/api'
import { getPricing } from '../pricing/api'
import type { PricingModel } from '../pricing/types'

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
  } else if (
    pricingQuery.isError ||
    groupsQuery.isError ||
    groups.length === 0
  ) {
    content = (
      <Alert variant='destructive'>
        <AlertTitle>{t('Image catalog unavailable')}</AlertTitle>
        <AlertDescription>
          {t(
            'No image-capable model and routing group is currently available.'
          )}
        </AlertDescription>
      </Alert>
    )
  } else {
    content = (
      <div className='grid gap-10 lg:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]'>
        <section
          className='grid content-start gap-5'
          aria-labelledby='drawing-prompt'
        >
          <div className='grid gap-2'>
            <Label htmlFor='drawing-prompt'>{t('Describe an image')}</Label>
            <Textarea
              id='drawing-prompt'
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder={t('Describe what you want to see...')}
              maxLength={2000}
              rows={7}
            />
            <p className='text-muted-foreground text-xs'>
              {prompt.length}/2000
            </p>
          </div>
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
            <p className='text-muted-foreground text-xs'>
              {t('Billing follows the selected group configuration.')}
            </p>
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
          <div className='grid grid-cols-2 gap-3'>
            <div className='grid gap-2'>
              <Label htmlFor='drawing-size'>{t('Size (optional)')}</Label>
              <Input
                id='drawing-size'
                value={size}
                onChange={(event) => setSize(event.target.value)}
                placeholder='1024x1024'
                maxLength={32}
              />
            </div>
            <div className='grid gap-2'>
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
          </div>
          <div className='grid gap-2'>
            <Label htmlFor='drawing-quality'>{t('Quality (optional)')}</Label>
            <Input
              id='drawing-quality'
              value={quality}
              onChange={(event) => setQuality(event.target.value)}
              maxLength={32}
            />
          </div>
          {error ? <p className='text-destructive text-sm'>{error}</p> : null}
          <Button
            type='button'
            className='w-full sm:w-auto'
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
        </section>
        <section className='min-w-0' aria-live='polite'>
          {results.length > 0 ? (
            <div className='grid gap-5 sm:grid-cols-2'>
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
            <div className='text-muted-foreground flex min-h-72 items-center justify-center text-center text-sm'>
              {t('Your generated images will appear here.')}
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
