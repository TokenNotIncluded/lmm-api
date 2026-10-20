/*
Copyright (C) 2026 LIghtJUNction
*/
import {
  Cancel01Icon,
  Image01Icon,
  ImageAdd01Icon,
  Loading03Icon,
  McpServerIcon,
  SparklesIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button, buttonVariants } from '@/components/ui/button'
import {
  Drawer,
  DrawerContent,
  DrawerHeader,
  DrawerTitle,
} from '@/components/ui/drawer'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Kbd } from '@/components/ui/kbd'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { copyToClipboard } from '@/lib/copy-to-clipboard'
import { useAuthStore } from '@/stores/auth-store'

import { getPricing, pricingQueryKey } from '../pricing/api'
import type { PricingModel } from '../pricing/types'
import { marketAPI, type CallResponse } from '../tool-market/api'
import { DrawingErrorBoundary } from './drawing-error-boundary'
import { DrawingGallery } from './drawing-gallery'
import { DrawingSnakeGame } from './drawing-snake-game'
import {
  getActiveDrawingTask,
  getDrawingDraft,
  getDrawingMinigamePref,
  saveDrawingDraft,
  setDrawingMinigamePref,
  subscribeActiveDrawingTask,
  updateActiveDrawingTask,
  type ActiveDrawingTask,
} from './drawing-task-state'
import {
  getDrawingRequestErrorKind,
  getDrawingRequestErrorMessage,
  getDrawingRequestStatus,
} from './error-state'
import { DRAWING_HISTORY_BYTES, DRAWING_HISTORY_LIMIT } from './history-storage'
import { drawingSource } from './image-bytes'
import {
  authorizeDrawingOnce,
  discoverDrawingTool,
  drawingMultipartBody,
  drawingStructuredContent,
  postDrawingMultipart,
} from './market-drawing'
import {
  clearDrawingOperation,
  readDrawingOperation,
  saveDrawingOperation,
  settledImageSchema,
  imageConfirmationURL,
  imagePreviewExpiry,
  type DrawingOperation,
} from './native-operation'
import { PromptIdeaButton } from './prompt-idea-button'
import { applyPromptIdea } from './prompt-inspiration'
import { useDrawingHistory } from './use-drawing-history'

type ReferenceImage = {
  id: string
  file: File
  previewUrl: string
}

const maxReferenceImages = 8
const maxReferenceImageBytes = 10 * 1024 * 1024
const maxTotalReferenceImageBytes = 16 * 1024 * 1024
const supportedReferenceImageTypes = ['image/jpeg', 'image/png', 'image/webp']
const chineseImageOrdinals = ['一', '二', '三', '四', '五', '六', '七', '八']

function isImageModel(model: PricingModel): boolean {
  return (model.image_variants?.length ?? 0) > 0
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
  const userId = useAuthStore((state) => state.auth.user?.id)
  // Remount every account-owned state, including prompt/reference files.
  // Never render one account's previews for another.
  return userId ? (
    <DrawingErrorBoundary>
      <DrawingWorkbench key={userId} userId={userId} />
    </DrawingErrorBoundary>
  ) : null
}

function DrawingWorkbench({ userId }: { userId: number }) {
  const { t, i18n } = useTranslation()
  const history = useDrawingHistory(userId)
  const results = history.images
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const [operation, setOperation] = useState(() => readDrawingOperation(userId))
  const [recovering, setRecovering] = useState(false)
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000))
  useEffect(() => {
    if (!operation?.confirmationExpiresAt) return
    const timer = setInterval(
      () => setNowUnix(Math.floor(Date.now() / 1000)),
      1000
    )
    return () => clearInterval(timer)
  }, [operation?.confirmationExpiresAt])
  const activeRef = useRef(true)
  const requestPendingRef = useRef(false)
  const currentAbortRef = useRef<AbortController | null>(null)
  const isCurrentUser = useCallback(
    () =>
      activeRef.current &&
      useAuthStore.getState().auth.user?.id === userId &&
      useAuthStore.getState().auth.session?.sid === sessionId,
    [userId, sessionId]
  )
  useEffect(() => {
    activeRef.current = true
    return () => {
      activeRef.current = false
    }
  }, [])

  const initialDraft = useMemo(() => getDrawingDraft(userId), [userId])
  const [prompt, setPrompt] = useState(initialDraft.prompt || '')
  const [group, setGroup] = useState(initialDraft.group || '')
  const [model, setModel] = useState(initialDraft.model || '')
  const [size, setSize] = useState(initialDraft.size || '')
  const [quality, setQuality] = useState(initialDraft.quality || '')
  const [count, setCount] = useState(initialDraft.count || '1')
  const [referenceImages, setReferenceImages] = useState<ReferenceImage[]>([])
  const [error, setError] = useState<string | null>(null)
  const [errorStatus, setErrorStatus] = useState<number | null>(null)
  const [stoppedMessage, setStoppedMessage] = useState<string | null>(null)
  const [generating, setGenerating] = useState(false)
  const [elapsedSeconds, setElapsedSeconds] = useState(0)
  const [minigameEnabled, setMinigameEnabled] = useState(getDrawingMinigamePref)
  const [minigameExpanded, setMinigameExpanded] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [activeTask, setActiveTask] = useState<ActiveDrawingTask | null>(() =>
    getActiveDrawingTask(userId)
  )

  const referenceInputRef = useRef<HTMLInputElement>(null)
  const promptInputRef = useRef<HTMLTextAreaElement>(null)
  const previewUrlsRef = useRef(new Set<string>())

  // Save prompt draft automatically
  useEffect(() => {
    saveDrawingDraft(userId, { prompt })
  }, [userId, prompt])

  // Subscribe to background active task across SPA navigations
  useEffect(() => {
    const existing = getActiveDrawingTask(userId)
    const syncTask = (task: ActiveDrawingTask | null) => {
      if (!isCurrentUser()) return
      setActiveTask(task)
      if (task && task.status === 'generating') {
        setGenerating(!task.waitingStopped)
        setStoppedMessage(
          task.waitingStopped
            ? t(
                'Generation continues in this tab. You can switch pages; do not reload or close this tab until the result is saved.'
              )
            : null
        )
        if (task.abortController) {
          currentAbortRef.current = task.abortController
        }
      } else if (task && task.status === 'failed') {
        setGenerating(false)
        setStoppedMessage(null)
        if (task.error) setError(task.error)
        if (task.errorStatus) setErrorStatus(task.errorStatus)
      } else if (
        !task ||
        task.status === 'completed' ||
        task.status === 'stopped'
      ) {
        setGenerating(false)
        setStoppedMessage(null)
      }
    }
    syncTask(existing)
    return subscribeActiveDrawingTask(userId, syncTask)
  }, [userId, isCurrentUser, t])

  // Live elapsed timer
  useEffect(() => {
    if (!generating) {
      setElapsedSeconds(0)
      return
    }
    const started = activeTask?.startedAt || Date.now()
    const updateTime = () => {
      const sec = Math.max(1, Math.floor((Date.now() - started) / 1000))
      setElapsedSeconds(sec)
    }
    updateTime()
    const timer = setInterval(updateTime, 1000)
    return () => clearInterval(timer)
  }, [generating, activeTask?.startedAt])

  // Offer once per generation; closing a game must not immediately reopen it.
  const gameOfferedRef = useRef(false)
  useEffect(() => {
    if (!generating) {
      gameOfferedRef.current = false
      setMinigameExpanded(false)
      return
    }
    if (elapsedSeconds >= 3 && minigameEnabled && !gameOfferedRef.current) {
      gameOfferedRef.current = true
      setMinigameExpanded(true)
    }
  }, [generating, elapsedSeconds, minigameEnabled])

  const pricingQuery = useQuery({
    queryKey: pricingQueryKey(userId, sessionId),
    queryFn: ({ signal }) => getPricing(signal),
    staleTime: 60_000,
    retry: false,
  })
  const drawingToolQuery = useQuery({
    queryKey: ['drawing-market-tool', userId, sessionId],
    queryFn: discoverDrawingTool,
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
  const groups = useMemo(
    () =>
      [
        ...new Set(
          imageModels.flatMap((item) =>
            (item.image_variants ?? []).map((variant) => variant.group)
          )
        ),
      ].sort(),
    [imageModels]
  )
  const selectedGroup = groups.includes(group)
    ? group
    : groups.includes('image-2')
      ? 'image-2'
      : (groups[0] ?? '')
  const modelsForGroup = imageModels.filter((item) =>
    item.image_variants?.some((variant) => variant.group === selectedGroup)
  )
  const selectedModel = modelsForGroup.some((item) => item.model_name === model)
    ? model
    : modelsForGroup.some((item) => item.model_name === 'image-2')
      ? 'image-2'
      : (modelsForGroup[0]?.model_name ?? '')
  const groupDescription: string | undefined = undefined
  const accessGranted = drawingToolQuery.isSuccess
  const hasPrompt = prompt.trim().length > 0
  const submitShortcutLabel =
    typeof navigator !== 'undefined' &&
    /mac|iphone|ipad/i.test(navigator.userAgent)
      ? '⌘↵'
      : 'Ctrl+↵'
  const variants =
    modelsForGroup
      .find((item) => item.model_name === selectedModel)
      ?.image_variants?.filter(
        (variant) =>
          variant.group === selectedGroup &&
          variant.endpoint ===
            (referenceImages.length ? 'edits' : 'generations')
      ) ?? []
  const sizes = [...new Set(variants.map((variant) => variant.size))]
  const effectiveSize = sizes.includes(size) ? size : (sizes[0] ?? '')
  const qualities = [
    ...new Set(
      variants
        .filter((variant) => variant.size === effectiveSize)
        .map((variant) => variant.quality)
    ),
  ]
  const sizePresets = sizes.map((value) => ({
    value,
    label: value || t('Default'),
  }))
  const qualityPresets = qualities.map((value) => ({
    value,
    label: value || t('Default'),
  }))

  const selectedSize = sizePresets.some((option) => option.value === size)
    ? size
    : (sizes[0] ?? '')
  const selectedQuality = qualityPresets.some(
    (option) => option.value === quality
  )
    ? quality
    : (qualities[0] ?? '')
  const maxCount = Math.min(
    4,
    ...variants
      .filter(
        (variant) =>
          variant.size === selectedSize && variant.quality === selectedQuality
      )
      .map((variant) => variant.max_n)
  )
  const selectedCount =
    Number.isSafeInteger(Number(count)) &&
    Number(count) >= 1 &&
    Number(count) <= maxCount
      ? Number(count)
      : 1

  const selectedVariant = variants.find(
    (variant) =>
      variant.size === selectedSize && variant.quality === selectedQuality
  )
  const estimateCredits = selectedVariant
    ? (() => {
        const numerator =
          BigInt(selectedVariant.credits_per_image.numerator) *
          BigInt(selectedCount)
        const denominator = BigInt(
          selectedVariant.credits_per_image.denominator
        )
        return ((numerator + denominator - 1n) / denominator).toString()
      })()
    : null

  const configurationReady = Boolean(
    selectedGroup && selectedModel && variants.length
  )

  useEffect(
    () => () => {
      for (const previewUrl of previewUrlsRef.current) {
        URL.revokeObjectURL(previewUrl)
      }
      previewUrlsRef.current.clear()
    },
    []
  )

  const referenceImageLabel = (index: number) => {
    const language = i18n.resolvedLanguage ?? i18n.language
    const ordinal = language.startsWith('zh')
      ? (chineseImageOrdinals[index] ?? String(index + 1))
      : String(index + 1)
    return t('Reference image {{index}}', { index: ordinal })
  }

  const addReferenceImages = (files: FileList | null) => {
    const selected = [...(files ?? [])]
    if (selected.length === 0) return
    if (referenceImages.length + selected.length > maxReferenceImages) {
      setError(
        t('You can upload up to {{count}} reference images.', {
          count: maxReferenceImages,
        })
      )
      return
    }
    const unsupported = selected.find(
      (file) => !supportedReferenceImageTypes.includes(file.type)
    )
    if (unsupported) {
      setError(
        t('{{name}} is not a supported image file.', {
          name: unsupported.name,
        })
      )
      return
    }
    const oversized = selected.find(
      (file) => file.size > maxReferenceImageBytes
    )
    if (oversized) {
      setError(
        t('{{name}} exceeds the {{size}} MB limit.', {
          name: oversized.name,
          size: maxReferenceImageBytes / 1024 / 1024,
        })
      )
      return
    }
    const totalBytes = [
      ...referenceImages.map((image) => image.file),
      ...selected,
    ].reduce((total, file) => total + file.size, 0)
    if (totalBytes > maxTotalReferenceImageBytes) {
      setError(t('Reference images must be at most 16 MiB in total.'))
      return
    }
    const additions = selected.map((file) => {
      const previewUrl = URL.createObjectURL(file)
      previewUrlsRef.current.add(previewUrl)
      return { id: crypto.randomUUID(), file, previewUrl }
    })
    setReferenceImages((current) => [...current, ...additions])
    setError(null)
  }

  const removeReferenceImage = (id: string) => {
    const target = referenceImages.find((image) => image.id === id)
    if (target) {
      URL.revokeObjectURL(target.previewUrl)
      previewUrlsRef.current.delete(target.previewUrl)
    }
    setReferenceImages((current) => current.filter((image) => image.id !== id))
  }

  useEffect(() => {
    if (selectedGroup) {
      saveDrawingDraft(userId, {
        group: selectedGroup,
        model: selectedModel,
        size: selectedSize,
        quality: selectedQuality,
        count,
      })
    }
  }, [
    userId,
    selectedGroup,
    selectedModel,
    selectedSize,
    selectedQuality,
    count,
  ])

  const stopGeneration = () => {
    updateActiveDrawingTask(userId, { waitingStopped: true })
    setGenerating(false)
    setError(null)
    setErrorStatus(null)
    setStoppedMessage(
      t(
        'Generation continues in this tab. You can switch pages; do not reload or close this tab until the result is saved.'
      )
    )
  }

  const copyErrorDetails = async () => {
    const details = {
      timestamp: new Date().toISOString(),
      model: selectedModel,
      group: selectedGroup,
      prompt: prompt.trim(),
      status: errorStatus,
      error,
    }
    const success = await copyToClipboard(JSON.stringify(details, null, 2))
    if (success) {
      toast.success(t('Error details copied'))
    }
  }

  const acceptReceipt = async (
    receipt: CallResponse,
    original: DrawingOperation,
    ticket: number
  ) => {
    if (
      receipt.call.tool_id !== original.toolId ||
      receipt.call.version_id !== original.versionId ||
      receipt.call.grant_id !== original.grantId ||
      receipt.call.client_id !== 'web-market' ||
      receipt.call.user_id !== original.userId ||
      (original.requestState &&
        receipt.request_state &&
        original.requestState !== receipt.request_state) ||
      (original.callId && receipt.call.id !== original.callId)
    ) {
      throw new Error('Image request identity did not match')
    }
    const content =
      receipt.result === undefined
        ? undefined
        : drawingStructuredContent(receipt.result)
    const expiresAt = imagePreviewExpiry(content)
    if (
      original.confirmationExpiresAt &&
      expiresAt &&
      expiresAt !== original.confirmationExpiresAt
    ) {
      throw new Error('Image confirmation expiry did not match')
    }
    const next: DrawingOperation = {
      ...original,
      callId: receipt.call.id,
      mayContinue: receipt.call.execution_status === 'awaiting_confirmation',
      confirmationExpiresAt: expiresAt ?? original.confirmationExpiresAt,
      requestState: receipt.request_state ?? original.requestState,
      confirmationURL:
        imageConfirmationURL(content, window.location.origin) ??
        original.confirmationURL,
    }
    saveDrawingOperation(next)
    if (isCurrentUser()) setOperation(next)
    if (
      receipt.call.settlement_status === 'settled' &&
      receipt.result_expired
    ) {
      clearDrawingOperation(userId, next.requestId)
      if (isCurrentUser()) {
        setOperation(null)
        setError(
          t(
            'The saved result has expired. The call and transfer records are retained.'
          )
        )
      }
      return
    }
    if (receipt.call.settlement_status === 'settled') {
      const result = settledImageSchema.parse(content)
      const usable = result.output.data.filter(
        (image) => drawingSource(image) !== undefined
      )
      if (!usable.length) throw new Error(t('No images were returned'))
      await history.remember(
        usable,
        {
          prompt: next.prompt,
          model: next.model,
          group: next.group,
          createdAt: next.createdAt,
          size: next.size,
          quality: next.quality,
          count: String(next.count),
        },
        ticket
      )
      clearDrawingOperation(userId, next.requestId)
      if (isCurrentUser()) {
        setOperation(null)
        toast.success(t('Image generation completed'))
      }
    } else if (
      receipt.call.settlement_status === 'released' &&
      ['failed', 'cancelled'].includes(receipt.call.execution_status)
    ) {
      clearDrawingOperation(userId, next.requestId)
      if (isCurrentUser()) {
        setOperation(null)
        setError(t('Unable to generate the image'))
      }
    }
  }

  const recoverOperation = async (resume: boolean) => {
    const original = readDrawingOperation(userId)
    if (
      !original ||
      recovering ||
      !isCurrentUser() ||
      (resume && !original.mayContinue)
    ) {
      return
    }
    setRecovering(true)
    const ticket = history.capture()
    setError(null)
    try {
      const receipt =
        resume && original.callId && original.requestState
          ? await marketAPI.resume(original.callId, {
              request_state: original.requestState,
            })
          : original.callId
            ? await marketAPI.result(original.callId)
            : await marketAPI.lookupRequest(original.requestId)
      if (isCurrentUser()) await acceptReceipt(receipt, original, ticket)
    } catch (cause: unknown) {
      if (isCurrentUser()) {
        setError(
          getDrawingRequestErrorMessage(
            cause,
            t(
              'The image request is not confirmed. Check this request before starting another.'
            )
          )
        )
      }
    } finally {
      if (isCurrentUser()) setRecovering(false)
    }
  }

  const generate = async () => {
    const cleanPrompt = prompt.trim()
    if (
      requestPendingRef.current ||
      readDrawingOperation(userId) ||
      !isCurrentUser() ||
      !drawingToolQuery.data ||
      history.clearing ||
      !cleanPrompt ||
      !variants.length ||
      !selectedGroup ||
      !selectedModel
    ) {
      return
    }
    requestPendingRef.current = true
    setGenerating(true)
    setError(null)
    setErrorStatus(null)
    setStoppedMessage(null)
    const releaseHistory = history.retain()
    const ticket = history.capture()
    let original: DrawingOperation | undefined
    try {
      const grant = await authorizeDrawingOnce(drawingToolQuery.data)
      if (!isCurrentUser()) return
      original = {
        requestId: crypto.randomUUID(),
        userId,
        toolId: drawingToolQuery.data.tool_id,
        versionId: drawingToolQuery.data.version_id,
        grantId: grant.id,
        prompt: cleanPrompt,
        model: selectedModel,
        group: selectedGroup,
        size: selectedSize,
        quality: selectedQuality,
        count: selectedCount,
        referenceCount: referenceImages.length,
        createdAt: Date.now(),
      }
      const request = {
        tool_id: original.toolId,
        version_id: original.versionId,
        grant_id: original.grantId,
        request_id: original.requestId,
        arguments: {
          prompt: original.prompt,
          model: original.model,
          group: original.group,
          n: original.count,
          ...(original.size ? { size: original.size } : {}),
          ...(original.quality ? { quality: original.quality } : {}),
        },
      }
      const form = referenceImages.length
        ? await drawingMultipartBody(
            request,
            referenceImages.map((image) => image.file)
          )
        : null
      if (!isCurrentUser()) return
      saveDrawingOperation(original)
      setOperation(original)
      const receipt = form
        ? await postDrawingMultipart(form)
        : await marketAPI.invoke(request)
      if (isCurrentUser()) await acceptReceipt(receipt, original, ticket)
    } catch (cause: unknown) {
      if (isCurrentUser()) {
        setError(
          getDrawingRequestErrorMessage(
            cause,
            original
              ? t(
                  'The image request is not confirmed. Check this request before starting another.'
                )
              : t('Unable to generate the image')
          )
        )
        setErrorStatus(getDrawingRequestStatus(cause))
      }
    } finally {
      releaseHistory()
      requestPendingRef.current = false
      if (isCurrentUser()) setGenerating(false)
    }
  }

  // Keyboard: ⌘/Ctrl+Enter submits. The listener is registered once and reads
  // the live handler through a ref, so it never needs to be torn down between
  // renders. generate() re-checks every precondition, so a stale call is safe.
  // ⌘/Ctrl+I belongs to PromptIdeaButton, which owns the roll it triggers.
  const generateRef = useRef(generate)
  generateRef.current = generate
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.repeat) return
      if (event.defaultPrevented) return
      if (event.key.toLowerCase() !== 'enter') return
      const target = event.target as HTMLElement | null
      if (target?.closest('[data-slot="drawing-inspector"], [role="dialog"]')) {
        return
      }
      event.preventDefault()
      void generateRef.current()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  let content: ReactNode
  const standaloneHistory = true
  if (drawingToolQuery.isLoading || pricingQuery.isLoading) {
    content = (
      <div className='grid gap-4 sm:grid-cols-2'>
        <Skeleton className='h-10 w-full' />
        <Skeleton className='h-10 w-full' />
        <Skeleton className='h-32 w-full sm:col-span-2' />
      </div>
    )
  } else if (drawingToolQuery.isError || pricingQuery.isError) {
    content = (
      <DrawingQueryErrorAlert
        title={t('Failed to load')}
        error={drawingToolQuery.error ?? pricingQuery.error}
        onRetry={() =>
          Promise.all([drawingToolQuery.refetch(), pricingQuery.refetch()])
        }
      />
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
              void Promise.all([
                pricingQuery.refetch(),
                drawingToolQuery.refetch(),
              ])
            }}
          >
            {t('Retry')}
          </Button>
        </AlertAction>
      </Alert>
    )
  } else {
    const settingsForm = (
      <div className='grid gap-5'>
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
              <NativeSelectOption key={item.model_name} value={item.model_name}>
                {item.model_name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
        <div className='grid gap-4 sm:grid-cols-2 xl:grid-cols-1'>
          <div className='grid gap-2'>
            <Label htmlFor='drawing-size'>{t('Size (optional)')}</Label>
            <NativeSelect
              id='drawing-size'
              value={selectedSize}
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
            <Label htmlFor='drawing-quality'>{t('Quality (optional)')}</Label>
            <NativeSelect
              id='drawing-quality'
              value={selectedQuality}
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
        {estimateCredits !== null ? (
          <p className='text-muted-foreground text-xs'>
            {t(
              'Estimated image cost: {{credits}} Credits. Final reservation is shown in the confirmation.',
              { credits: estimateCredits }
            )}
          </p>
        ) : null}
        <div className='grid max-w-40 gap-2'>
          <Label htmlFor='drawing-count'>{t('Images')}</Label>
          <NativeSelect
            id='drawing-count'
            value={String(selectedCount)}
            className='w-full'
            onChange={(event) => setCount(event.target.value)}
          >
            {Array.from({ length: maxCount }, (_, index) => index + 1).map(
              (value) => (
                <NativeSelectOption key={value} value={String(value)}>
                  {value}
                </NativeSelectOption>
              )
            )}
          </NativeSelect>
        </div>
      </div>
    )

    content = (
      <div className='grid gap-4 xl:grid-cols-[minmax(0,1fr)_19rem] xl:items-stretch'>
        <section
          data-slot='drawing-canvas'
          className='relative flex min-h-[20rem] min-w-0 flex-col overflow-hidden rounded-lg border border-white/10 bg-[#111210] sm:min-h-[26rem] xl:min-h-[36rem]'
          aria-live='polite'
          aria-labelledby='drawing-canvas-title'
        >
          <header className='flex items-center justify-between gap-3 border-b border-white/10 px-4 py-3 sm:px-5'>
            <div className='flex min-w-0 items-center gap-3'>
              <div className='bg-primary/15 text-primary flex size-8 shrink-0 items-center justify-center rounded-md'>
                <HugeiconsIcon
                  icon={SparklesIcon}
                  className='size-4'
                  strokeWidth={2}
                  aria-hidden='true'
                />
              </div>
              <h2
                id='drawing-canvas-title'
                className='truncate text-sm font-medium text-white/90'
              >
                {selectedModel || t('Preview')}
              </h2>
            </div>
            <div className='flex shrink-0 items-center gap-2'>
              {/* Mobile generation settings trigger */}
              <Button
                type='button'
                variant='outline'
                size='xs'
                className='border-white/15 bg-white/5 text-xs text-white/80 xl:hidden'
                onClick={() => setSettingsOpen(true)}
              >
                <span className='max-w-[7rem] truncate font-mono'>
                  {selectedModel || t('Preview')}
                </span>
                {selectedGroup ? (
                  <>
                    <span className='text-white/40'>·</span>
                    <span className='max-w-[5rem] truncate text-white/70'>
                      {selectedGroup}
                    </span>
                  </>
                ) : null}
                <span className='ml-1 text-xs'>⚙️</span>
              </Button>

              <Badge
                variant='outline'
                className='border-white/15 bg-white/5 text-white/80'
              >
                {generating
                  ? referenceImages.length > 0
                    ? t('Editing...')
                    : t('Generating...')
                  : results.length > 0
                    ? t('Preview')
                    : referenceImages.length > 0
                      ? t('Edit image')
                      : t('Draft')}
              </Badge>
              {selectedGroup ? (
                <Badge
                  variant='outline'
                  className='hidden border-white/15 text-white/70 sm:inline-flex'
                >
                  {selectedGroup}
                </Badge>
              ) : null}
            </div>
          </header>

          <div className='flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-4 sm:p-8'>
            {generating ? (
              <div className='mx-auto flex max-w-md flex-col items-center justify-center p-4 text-center text-white/90'>
                <div className='bg-primary/15 text-primary mb-3 flex size-12 items-center justify-center rounded-lg'>
                  <HugeiconsIcon
                    icon={Loading03Icon}
                    className='size-7 animate-spin'
                    strokeWidth={2}
                    aria-hidden='true'
                  />
                </div>
                <p className='text-sm font-medium'>
                  {t('Request submitted · Waiting {{seconds}}s', {
                    seconds: elapsedSeconds,
                  })}
                </p>
                <p className='mt-1 text-xs text-white/60'>
                  {t('Waiting for image generation result...')}
                </p>
                <p className='mt-1 max-w-xs text-[11px] text-white/40'>
                  {t(
                    'Generation continues in this tab. You can switch pages; do not reload or close this tab until the result is saved.'
                  )}
                </p>

                {/* Minigame section */}
                {elapsedSeconds >= 3 ? (
                  <div className='mt-4 flex w-full flex-col items-center'>
                    {minigameExpanded ? (
                      <div className='flex flex-col items-center gap-2'>
                        <DrawingSnakeGame />
                        <Button
                          type='button'
                          variant='ghost'
                          size='xs'
                          className='mt-1 text-white/60 hover:text-white'
                          onClick={() => setMinigameExpanded(false)}
                        >
                          {t('Collapse minigame')}
                        </Button>
                      </div>
                    ) : (
                      <Button
                        type='button'
                        variant='outline'
                        size='xs'
                        className='border-white/15 bg-white/5 text-white hover:bg-white/10'
                        onClick={() => {
                          setMinigameExpanded(true)
                          setMinigameEnabled(true)
                          setDrawingMinigamePref(true)
                        }}
                      >
                        {t('Play dot-matrix snake while waiting')}
                      </Button>
                    )}
                  </div>
                ) : null}

                <div className='mt-5 flex items-center gap-3'>
                  <Button
                    type='button'
                    variant='outline'
                    size='sm'
                    className='border-white/20 bg-white/5 text-white hover:bg-white/15'
                    onClick={stopGeneration}
                  >
                    {t('Stop waiting')}
                  </Button>
                </div>
              </div>
            ) : stoppedMessage ? (
              <div className='w-full max-w-md p-4 text-center'>
                <Empty className='max-w-md text-white'>
                  <EmptyHeader>
                    <EmptyMedia
                      variant='icon'
                      className='size-10 bg-white/10 text-white/75'
                    >
                      <HugeiconsIcon
                        icon={Cancel01Icon}
                        className='size-5'
                        strokeWidth={2}
                        aria-hidden='true'
                      />
                    </EmptyMedia>
                    <EmptyTitle className='text-white/85'>
                      {t('Stop waiting')}
                    </EmptyTitle>
                    <EmptyDescription className='mt-2 text-xs text-white/65'>
                      {stoppedMessage}
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </div>
            ) : results.length > 0 ? (
              <DrawingGallery images={results} />
            ) : (
              <Empty className='max-w-md text-white'>
                <EmptyHeader>
                  <EmptyMedia
                    variant='icon'
                    className='size-10 bg-white/10 text-white/75'
                  >
                    <HugeiconsIcon
                      icon={Image01Icon}
                      className='size-5'
                      strokeWidth={2}
                      aria-hidden='true'
                    />
                  </EmptyMedia>
                  <EmptyTitle className='text-white/85'>
                    {t('Your generated images will appear here.')}
                  </EmptyTitle>
                  <EmptyDescription className='text-xs text-white/65'>
                    {hasPrompt
                      ? t(
                          'Review the generated images here when the request finishes.'
                        )
                      : t(
                          'Describe an image, choose a group, and generate a preview.'
                        )}
                  </EmptyDescription>
                </EmptyHeader>
                {hasPrompt ? null : (
                  <EmptyContent>
                    <Button
                      type='button'
                      size='sm'
                      data-testid='drawing-empty-cta'
                      className='border-white/15 bg-white/10 text-white hover:bg-white/20 hover:text-white'
                      onClick={() => promptInputRef.current?.focus()}
                    >
                      <HugeiconsIcon
                        icon={SparklesIcon}
                        data-icon='inline-start'
                        strokeWidth={2}
                        aria-hidden='true'
                      />
                      {t('Write your first prompt')}
                    </Button>
                  </EmptyContent>
                )}
              </Empty>
            )}
          </div>

          <div
            data-slot='drawing-composer'
            className='border-t border-white/10 bg-black/20 p-3 sm:p-4'
          >
            <div className='rounded-lg border border-white/10 bg-black/35 p-3 sm:p-4'>
              <div className='mb-2 flex items-center justify-between gap-3'>
                <Label htmlFor='drawing-prompt-input' className='text-white/85'>
                  {t('Prompt')}
                </Label>
                <div className='flex items-center gap-3'>
                  {hasPrompt ? (
                    <button
                      type='button'
                      disabled={generating}
                      className='rounded-sm text-xs text-white/60 underline-offset-2 transition-colors hover:text-white/90 hover:underline focus-visible:ring-2 focus-visible:ring-white/40 focus-visible:outline-none'
                      onClick={() => setPrompt('')}
                    >
                      {t('Clear')}
                    </button>
                  ) : null}
                  <span className='text-xs text-white/60 tabular-nums'>
                    {prompt.length}/2000
                  </span>
                  <span className='hidden items-center gap-1 text-xs text-white/45 md:flex'>
                    {t('to generate')}
                    <Kbd className='bg-white/10 text-white/70'>
                      {submitShortcutLabel}
                    </Kbd>
                  </span>
                </div>
              </div>
              <Textarea
                ref={promptInputRef}
                id='drawing-prompt-input'
                value={prompt}
                onChange={(event) => setPrompt(event.target.value)}
                placeholder={t('Describe what you want to see...')}
                maxLength={2000}
                rows={3}
                style={{ backgroundColor: 'transparent' }}
                className='min-h-20 resize-none border-0 bg-transparent px-0 py-1 text-base text-white shadow-none placeholder:text-white/50 focus-visible:ring-0'
              />

              <input
                ref={referenceInputRef}
                id='drawing-reference-images'
                type='file'
                accept={supportedReferenceImageTypes.join(',')}
                multiple
                disabled={generating}
                className='sr-only'
                onChange={(event) => {
                  addReferenceImages(event.target.files)
                  event.target.value = ''
                }}
              />

              {referenceImages.length > 0 ? (
                <div className='mt-3 border-t border-white/10 pt-3'>
                  <div className='mb-2 flex items-center justify-between gap-3'>
                    <Label
                      htmlFor='drawing-reference-images'
                      className='text-xs text-white/75'
                    >
                      {t('Reference images')}
                    </Label>
                    <span className='text-xs text-white/60 tabular-nums'>
                      {referenceImages.length}/{maxReferenceImages}
                    </span>
                  </div>
                  <div className='flex gap-2 overflow-x-auto pb-1'>
                    {referenceImages.map((image, index) => {
                      const label = referenceImageLabel(index)
                      return (
                        <figure
                          key={image.id}
                          className='group relative size-16 shrink-0 overflow-hidden rounded-md border border-white/15 bg-black/30 sm:size-20'
                          title={image.file.name}
                        >
                          <img
                            src={image.previewUrl}
                            alt={label}
                            className='size-full object-cover'
                          />
                          <figcaption className='absolute inset-x-1 bottom-1 truncate rounded-sm bg-black/80 px-1.5 py-0.5 text-[10px] font-medium text-white'>
                            {label}
                          </figcaption>
                          <Button
                            type='button'
                            variant='secondary'
                            size='icon-xs'
                            className='absolute top-1 right-1 bg-black/80 text-white hover:bg-black'
                            disabled={generating}
                            aria-label={t('Remove {{name}}', { name: label })}
                            onClick={() => removeReferenceImage(image.id)}
                          >
                            <HugeiconsIcon
                              icon={Cancel01Icon}
                              strokeWidth={2}
                              aria-hidden='true'
                            />
                          </Button>
                        </figure>
                      )
                    })}
                  </div>
                  <p className='mt-2 text-xs leading-5 text-white/65'>
                    {t(
                      'Use the image labels in your prompt to describe how each reference should be used.'
                    )}
                  </p>
                </div>
              ) : null}

              {error ? (
                <Alert variant='destructive' className='mt-3'>
                  <AlertTitle className='flex items-center justify-between'>
                    <span>{t('Request failed')}</span>
                    {errorStatus ? (
                      <Badge
                        variant='outline'
                        className='text-destructive-foreground font-mono text-xs'
                      >
                        HTTP {errorStatus}
                      </Badge>
                    ) : null}
                  </AlertTitle>
                  <AlertDescription>{error}</AlertDescription>
                  <AlertAction className='gap-2'>
                    <Button
                      type='button'
                      size='sm'
                      variant='secondary'
                      onClick={() => void copyErrorDetails()}
                    >
                      {t('Copy error details')}
                    </Button>
                  </AlertAction>
                </Alert>
              ) : null}

              <div className='mt-3 flex flex-col gap-3 border-t border-white/10 pt-3 sm:flex-row sm:items-center sm:justify-between'>
                <div className='flex min-w-0 flex-1 items-center gap-2 sm:gap-3'>
                  <Button
                    type='button'
                    variant='outline'
                    size='sm'
                    className='shrink-0 border-white/15 bg-white/5 text-white hover:bg-white/10 hover:text-white'
                    disabled={
                      generating || referenceImages.length >= maxReferenceImages
                    }
                    onClick={() => referenceInputRef.current?.click()}
                  >
                    <HugeiconsIcon
                      icon={ImageAdd01Icon}
                      data-icon='inline-start'
                      strokeWidth={2}
                      aria-hidden='true'
                    />
                    <span className='hidden sm:inline'>
                      {t('Add reference images')}
                    </span>
                    <span className='sm:hidden'>{t('Reference')}</span>
                  </Button>
                  <PromptIdeaButton
                    prompt={prompt}
                    disabled={generating}
                    onApply={(idea) =>
                      setPrompt((current) => applyPromptIdea(current, idea))
                    }
                  />
                  <span className='hidden min-w-0 truncate text-xs text-white/65 lg:block'>
                    {referenceImages.length > 0
                      ? t(
                          'Reference images switch this request to image editing.'
                        )
                      : t('Be specific about the subject, mood, and style.')}
                  </span>
                </div>
                {generating ? (
                  <Button
                    type='button'
                    size='lg'
                    variant='outline'
                    className='w-full border-white/20 text-white hover:bg-white/10 sm:w-auto sm:min-w-36'
                    onClick={stopGeneration}
                  >
                    <HugeiconsIcon
                      icon={Cancel01Icon}
                      data-icon='inline-start'
                      strokeWidth={2}
                      aria-hidden='true'
                    />
                    {t('Stop waiting')}
                  </Button>
                ) : (
                  <Button
                    type='button'
                    size='lg'
                    className='w-full sm:w-auto sm:min-w-36'
                    onClick={() => void generate()}
                    disabled={
                      generating ||
                      activeTask?.status === 'generating' ||
                      operation !== null ||
                      history.clearing ||
                      !prompt.trim() ||
                      !selectedGroup ||
                      !selectedModel ||
                      !configurationReady
                    }
                  >
                    <HugeiconsIcon
                      icon={Image01Icon}
                      data-icon='inline-start'
                      strokeWidth={2}
                      aria-hidden='true'
                    />
                    {referenceImages.length > 0
                      ? t('Edit image')
                      : t('Generate image')}
                  </Button>
                )}
              </div>
            </div>
          </div>
        </section>

        <aside
          data-slot='drawing-inspector'
          className='bg-card flex min-w-0 flex-col rounded-lg border'
        >
          <div className='border-b p-4'>
            <div className='flex items-start justify-between gap-3'>
              <h2 className='text-base font-semibold'>
                {t('Generation setup')}
              </h2>
              {configurationReady ? (
                <Badge variant='secondary'>{t('Ready')}</Badge>
              ) : null}
            </div>
            <p className='text-muted-foreground mt-2 text-xs leading-5'>
              {t('Choose a route and output settings.')}
            </p>
          </div>

          <div className='p-4'>{settingsForm}</div>

          <div className='mt-auto border-t p-4'>
            <p className='text-muted-foreground text-xs leading-5'>
              {t('Billing follows the selected group configuration.')}
            </p>
          </div>
        </aside>

        {/* Mobile settings drawer */}
        <Drawer open={settingsOpen} onOpenChange={setSettingsOpen}>
          <DrawerContent className='max-h-[85vh] overflow-y-auto p-4 pb-8'>
            <DrawerHeader className='px-0 pt-0'>
              <DrawerTitle>{t('Generation settings')}</DrawerTitle>
            </DrawerHeader>
            <div className='mt-2'>{settingsForm}</div>
          </DrawerContent>
        </Drawer>
      </div>
    )
  }

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Drawing studio')}</SectionPageLayout.Title>
      {accessGranted ? (
        <SectionPageLayout.Actions>
          <Link
            to='/keys'
            className={buttonVariants({ variant: 'outline', size: 'sm' })}
          >
            {t('Prepare image-2 API Key')}
          </Link>
          <Link
            to='/keys'
            className={buttonVariants({ variant: 'outline', size: 'sm' })}
          >
            {t('Manage API Keys')}
          </Link>
          <Link
            to='/tool-market'
            className={buttonVariants({ variant: 'outline', size: 'sm' })}
          >
            <HugeiconsIcon
              icon={McpServerIcon}
              data-icon='inline-start'
              strokeWidth={2}
              aria-hidden='true'
            />
            {t('Tool market')}
          </Link>
        </SectionPageLayout.Actions>
      ) : null}
      <SectionPageLayout.Content>
        <div className='mx-auto w-full max-w-7xl pb-16'>
          <header className='mb-4 grid gap-2'>
            <p className='text-muted-foreground text-sm'>
              {t(
                'Create images through the same safe, group-aware relay used by the API.'
              )}{' '}
              {t(
                'This action authorizes one image tool call. Review and confirm the image price before execution.'
              )}
            </p>
          </header>
          {operation ? (
            <Alert className='mb-4' data-slot='drawing-operation'>
              <AlertTitle>
                {t('Review the image quote, then return here to continue.')}
              </AlertTitle>
              <AlertDescription>
                <p>
                  {t(
                    'The image request is not confirmed. Check this request before starting another.'
                  )}
                </p>
                {operation.confirmationExpiresAt ? (
                  <p>
                    {operation.confirmationExpiresAt > nowUnix
                      ? t(
                          'Image confirmation expires in {{seconds}} seconds.',
                          { seconds: operation.confirmationExpiresAt - nowUnix }
                        )
                      : t(
                          'Image confirmation expired. Check the original request.'
                        )}
                  </p>
                ) : null}
                <div className='flex flex-wrap gap-2'>
                  {operation.confirmationURL ? (
                    <a
                      href={
                        imageConfirmationURL(
                          { confirmation_url: operation.confirmationURL },
                          window.location.origin
                        ) ?? undefined
                      }
                      target='_blank'
                      rel='noopener noreferrer'
                      className={buttonVariants({
                        variant: 'outline',
                        size: 'sm',
                      })}
                    >
                      {t('Open image confirmation')}
                    </a>
                  ) : null}
                  <Button
                    size='sm'
                    variant='outline'
                    disabled={recovering || generating}
                    onClick={() => void recoverOperation(false)}
                  >
                    {t('Check the original request')}
                  </Button>
                  {operation.callId &&
                  operation.requestState &&
                  operation.mayContinue ? (
                    <Button
                      size='sm'
                      disabled={recovering || generating}
                      onClick={() => void recoverOperation(true)}
                    >
                      {t('Continue the original request')}
                    </Button>
                  ) : null}
                </div>
              </AlertDescription>
            </Alert>
          ) : null}
          <div
            className='mb-4 flex flex-wrap items-center justify-between gap-2'
            data-slot='drawing-history-controls'
          >
            <p className='text-muted-foreground max-w-3xl text-xs'>
              {t(
                'This browser keeps up to {{count}} images or {{size}} MB per account, newest first. Older images are removed automatically. Clearing browser data also removes this history.',
                {
                  count: DRAWING_HISTORY_LIMIT,
                  size: DRAWING_HISTORY_BYTES / 1024 / 1024,
                }
              )}
            </p>
            <Button
              type='button'
              size='sm'
              variant='outline'
              disabled={history.clearing}
              onClick={() => {
                if (
                  window.confirm(
                    t(
                      'Clear image history for this account in this browser? Pending results will not be saved.'
                    )
                  )
                ) {
                  void history.clear()
                }
              }}
            >
              {history.clearing
                ? t('Clearing history...')
                : t('Clear image history')}
            </Button>
          </div>
          {history.loading || history.saving ? (
            <p role='status' className='text-muted-foreground mb-3 text-xs'>
              {history.loading
                ? t('Loading image history...')
                : t('Saving image bytes in this browser...')}
            </p>
          ) : null}
          {history.warning ? (
            <Alert className='mb-4' data-slot='drawing-history-warning'>
              <AlertTitle>{t('Browser image history unavailable')}</AlertTitle>
              <AlertDescription>
                {history.warning === 'clear'
                  ? t(
                      'Browser history could not be cleared. Saved images may return after refresh. Try clearing history again; this will not generate or bill any images.'
                    )
                  : history.warning === 'load'
                    ? t(
                        'Saved image history could not be loaded. You can still generate images, but browser storage may be unavailable.'
                      )
                    : t(
                        'Generation succeeded, but some image bytes could not be saved in this browser (storage quota or image-host CORS restrictions). Download them now. Unsaved or URL-only previews may disappear after refresh. Do not regenerate to repair the cache; another generation is billed again.'
                      )}
              </AlertDescription>
            </Alert>
          ) : null}
          {content}
          {standaloneHistory && results.length > 0 ? (
            <section
              className='mt-4 rounded-lg bg-[#111210] p-4'
              aria-label={t('Image history')}
            >
              <DrawingGallery images={results} />
            </section>
          ) : null}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
