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
'use client'

import type { ToolUIPart } from 'ai'
import {
  CheckCircleIcon,
  ChevronDownIcon,
  CircleIcon,
  ClockIcon,
  WrenchIcon,
  XCircleIcon,
} from 'lucide-react'
import { type ComponentProps, isValidElement, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { cn } from '@/lib/utils'

import { CodeBlock } from './code-block'

// Workaround for missing types in 'ai' package
type ExtendedToolState =
  | ToolUIPart['state']
  | 'approval-requested'
  | 'approval-responded'
  | 'output-denied'

export type ToolProps = ComponentProps<typeof Collapsible>

export const Tool = ({ className, ...props }: ToolProps) => (
  <Collapsible
    className={cn('not-prose mb-4 w-full rounded-md border', className)}
    {...props}
  />
)

export type ToolHeaderProps = {
  title?: string
  type: ToolUIPart['type']
  state: ExtendedToolState
  summary?: string
  className?: string
}

const getStatusBadge = (
  status: ExtendedToolState,
  translate: (key: string) => string
) => {
  const labels: Record<ExtendedToolState, string> = {
    'input-streaming': translate('Pending'),
    'input-available': translate('Running'),
    'approval-requested': translate('Awaiting Approval'),
    'approval-responded': translate('Responded'),
    'output-available': translate('Completed'),
    'output-error': translate('Error'),
    'output-denied': translate('Denied'),
  }

  const icons: Record<ExtendedToolState, ReactNode> = {
    'input-streaming': <CircleIcon className='size-4' />,
    'input-available': <ClockIcon className='size-4 animate-pulse' />,
    'approval-requested': <ClockIcon className='text-warning size-4' />,
    'approval-responded': <CheckCircleIcon className='text-info size-4' />,
    'output-available': <CheckCircleIcon className='text-success size-4' />,
    'output-error': <XCircleIcon className='text-destructive size-4' />,
    'output-denied': <XCircleIcon className='text-warning size-4' />,
  }

  return (
    <Badge className='gap-1.5 text-xs' variant='secondary'>
      {icons[status]}
      {labels[status]}
    </Badge>
  )
}

export const ToolHeader = ({
  className,
  title,
  type,
  state,
  summary,
  ...props
}: ToolHeaderProps) => {
  const { t } = useTranslation()
  const toolIdentifier = type.startsWith('tool-') ? type.slice(5) : type
  return (
    <CollapsibleTrigger
      className={cn(
        'group flex w-full items-center justify-between gap-4 p-3 text-left',
        className
      )}
      {...props}
    >
      <div className='flex min-w-0 flex-1 items-start gap-2'>
        <WrenchIcon className='text-muted-foreground mt-0.5 size-4 shrink-0' />
        <div className='min-w-0'>
          <div className='flex flex-wrap items-center gap-2'>
            <span className='truncate text-sm font-medium'>
              {title ?? type.split('-').slice(1).join('-')}
            </span>
            <code className='text-muted-foreground max-w-full truncate rounded bg-black/5 px-1.5 py-0.5 text-[10px] dark:bg-white/10'>
              {toolIdentifier}
            </code>
            {getStatusBadge(state, t)}
          </div>
          {summary ? (
            <p className='text-muted-foreground mt-1 truncate text-xs'>
              {summary}
            </p>
          ) : null}
        </div>
      </div>
      <ChevronDownIcon className='text-muted-foreground size-4 shrink-0 transition-transform group-data-[panel-open]:rotate-180' />
    </CollapsibleTrigger>
  )
}

export type ToolContentProps = ComponentProps<typeof CollapsibleContent>

export const ToolContent = ({ className, ...props }: ToolContentProps) => (
  <CollapsibleContent
    className={cn(
      'data-closed:fade-out-0 data-closed:slide-out-to-top-2 data-open:slide-in-from-top-2 text-popover-foreground data-closed:animate-out data-open:animate-in outline-none',
      className
    )}
    {...props}
  />
)

export type ToolInputProps = ComponentProps<'div'> & {
  input: ToolUIPart['input']
}

export const ToolInput = ({ className, input, ...props }: ToolInputProps) => {
  const { t } = useTranslation()
  const entries = Object.entries(input ?? {})
  const formatValue = (value: unknown) => {
    if (typeof value === 'string') return value
    if (typeof value === 'object' && value !== null) {
      return JSON.stringify(value, null, 2)
    }
    return String(value)
  }
  return (
    <div className={cn('space-y-2 overflow-hidden p-4', className)} {...props}>
      <h4 className='text-muted-foreground text-xs font-medium tracking-wide uppercase'>
        {t('Parameters')}
      </h4>
      {entries.length > 0 ? (
        <dl className='bg-muted/50 divide-border grid gap-px divide-y overflow-hidden rounded-md'>
          {entries.map(([key, value]) => (
            <div
              key={key}
              className='grid gap-1 px-3 py-2 text-xs sm:grid-cols-[minmax(7rem,0.35fr)_minmax(0,1fr)] sm:gap-3'
            >
              <dt className='text-muted-foreground'>{key}</dt>
              <dd className='min-w-0 font-mono text-[11px] break-words whitespace-pre-wrap'>
                {formatValue(value)}
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className='bg-muted/50 text-muted-foreground rounded-md px-3 py-2 text-xs'>
          {t('No parameters')}
        </p>
      )}
    </div>
  )
}

export type ToolOutputProps = ComponentProps<'div'> & {
  output: ToolUIPart['output']
  errorText: ToolUIPart['errorText']
}

export const ToolOutput = ({
  className,
  output,
  errorText,
  ...props
}: ToolOutputProps) => {
  const { t } = useTranslation()
  if (!(output || errorText)) {
    return null
  }

  let Output = <div>{output as ReactNode}</div>

  if (typeof output === 'object' && !isValidElement(output)) {
    Output = (
      <CodeBlock code={JSON.stringify(output, null, 2)} language='json' />
    )
  } else if (typeof output === 'string') {
    Output = <p className='px-3 py-2 text-sm leading-5'>{output}</p>
  }

  return (
    <div className={cn('space-y-2 p-4', className)} {...props}>
      <h4 className='text-muted-foreground text-xs font-medium tracking-wide uppercase'>
        {errorText ? t('Error') : t('Result')}
      </h4>
      <div
        className={cn(
          'overflow-x-auto rounded-md text-xs [&_table]:w-full',
          errorText
            ? 'bg-destructive/10 text-destructive'
            : 'bg-muted/50 text-foreground'
        )}
      >
        {errorText && <div>{errorText}</div>}
        {Output}
      </div>
    </div>
  )
}
