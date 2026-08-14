/*
Copyright (C) 2026 LIghtJUNction
*/
import { ImageIcon, Sparkles } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'

import {
  generateAssistantImage,
  type AssistantGeneratedImage,
  type AssistantImageGenerationAction,
} from './api'

function imageSource(image: AssistantGeneratedImage): string | undefined {
  if (image.url?.trim()) return image.url.trim()
  if (image.b64_json?.trim()) return `data:image/png;base64,${image.b64_json}`
  return undefined
}

export function AssistantImageTool(props: {
  action: AssistantImageGenerationAction
}) {
  const { t } = useTranslation()
  const [generating, setGenerating] = useState(false)
  const [images, setImages] = useState<AssistantGeneratedImage[]>([])
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    setGenerating(false)
    setImages([])
    setFailed(false)
  }, [props.action.confirmation_token])

  const generate = async () => {
    if (generating || images.length > 0) return
    setGenerating(true)
    setFailed(false)
    try {
      const result = await generateAssistantImage(
        props.action.confirmation_token
      )
      if (result.length === 0) throw new Error(t('No images were returned'))
      setImages(result)
    } catch (error) {
      setFailed(true)
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to generate the image')
      )
    } finally {
      setGenerating(false)
    }
  }

  if (images.length > 0) {
    return (
      <div
        className='grid gap-3 border-t pt-4'
        data-testid='assistant-image-result'
      >
        <div className='flex items-center gap-2 text-sm font-medium'>
          <ImageIcon className='text-primary size-4' aria-hidden='true' />
          {t('Image generated')}
        </div>
        <div className='grid gap-3 sm:grid-cols-2'>
          {images.map((image) => {
            const src = imageSource(image)
            if (!src) return null
            return (
              <figure
                className='min-w-0'
                key={image.url ?? image.b64_json ?? image.revised_prompt}
              >
                <img
                  src={src}
                  alt={image.revised_prompt || props.action.prompt}
                  className='h-auto max-h-[28rem] w-full rounded-md object-contain'
                  loading='lazy'
                />
                {image.revised_prompt ? (
                  <figcaption className='text-muted-foreground mt-1 text-xs leading-5'>
                    {image.revised_prompt}
                  </figcaption>
                ) : null}
              </figure>
            )
          })}
        </div>
      </div>
    )
  }

  return (
    <div
      className='grid gap-3 border-t pt-4'
      data-testid='assistant-image-confirmation'
    >
      <div className='flex items-center gap-2 text-sm font-medium'>
        <Sparkles className='text-primary size-4' aria-hidden='true' />
        {t('Ready to generate an image')}
      </div>
      <p className='text-muted-foreground text-sm leading-6'>
        {t('Review the prompt and routing choice before generating.')}
      </p>
      <dl className='grid gap-2 text-sm sm:grid-cols-[auto_minmax(0,1fr)] sm:gap-x-4'>
        <dt className='text-muted-foreground'>{t('Prompt')}</dt>
        <dd className='min-w-0 break-words whitespace-pre-wrap'>
          {props.action.prompt}
        </dd>
        <dt className='text-muted-foreground'>{t('Model')}</dt>
        <dd className='truncate'>{props.action.model}</dd>
        <dt className='text-muted-foreground'>{t('Group')}</dt>
        <dd className='truncate'>{props.action.group}</dd>
        <dt className='text-muted-foreground'>{t('Images')}</dt>
        <dd>{props.action.n}</dd>
      </dl>
      {failed ? (
        <p className='text-destructive text-sm'>
          {t(
            'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.'
          )}
        </p>
      ) : null}
      <div>
        <Button
          type='button'
          onClick={() => void generate()}
          disabled={generating}
        >
          {generating ? <Spinner data-icon='inline-start' /> : null}
          {generating ? t('Generating...') : t('Generate image')}
        </Button>
      </div>
    </div>
  )
}
