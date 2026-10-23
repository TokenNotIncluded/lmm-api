/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useMemo, useState, type DragEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

import {
  errorMessage,
  operationId,
  priorityPreview,
  publish,
  undoPreview,
} from './api'
import type { Preview, PriorityChange, Target } from './contract'
import { applyPriorities, movePriority, priorityChanges } from './helpers'
import { PublicationReview } from './publication-review'
import { useCatalog } from './use-catalog'

export function NativePrioritySortButton() {
  const { t } = useTranslation(),
    [open, setOpen] = useState(false)
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger render={<Button variant='outline' />}>
        {t('Order native channel priorities')}
      </DialogTrigger>
      <DialogContent className='sm:max-w-5xl'>
        <DialogHeader>
          <DialogTitle>{t('Order native channel priorities')}</DialogTitle>
        </DialogHeader>
        {open && <NativePrioritySort />}
      </DialogContent>
    </Dialog>
  )
}
export function NativePrioritySort() {
  const { t } = useTranslation(),
    { scope, catalog, error: loadError, loading, reload } = useCatalog()
  const [routeId, setRouteId] = useState(''),
    [draft, setDraft] = useState<Target[]>(),
    [dragMode, setDragMode] = useState(false),
    [cards, setCards] = useState(false),
    [dragging, setDragging] = useState(''),
    [hover, setHover] = useState<PriorityChange[]>([]),
    [moved, setMoved] = useState(''),
    [anchor, setAnchor] = useState(''),
    [preview, setPreview] = useState<Preview>(),
    [operation, setOperation] = useState(''),
    [lastOperation, setLastOperation] = useState(''),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<unknown>(),
    [status, setStatus] = useState('')
  const route =
    catalog?.routes.find((r) => r.route_id === routeId) ?? catalog?.routes[0]
  const current = draft ?? route?.targets ?? [],
    changes = route ? priorityChanges(route.targets, current) : []
  const ordered = useMemo(
    () =>
      [...current].sort(
        (a, b) =>
          a.priority - b.priority ||
          (BigInt(a.target_id) < BigInt(b.target_id) ? -1 : 1)
      ),
    [current]
  )
  const projected = applyPriorities(current, hover)
  useEffect(() => {
    setDraft(undefined)
    setPreview(undefined)
    setHover([])
    setDragging('')
    setMoved('')
    setAnchor('')
  }, [catalog, routeId, scope])
  function place(
    movedId: string,
    anchorId: string,
    position: 'before' | 'same' | 'after'
  ) {
    const next = movePriority(current, movedId, anchorId, position)
    setDraft(applyPriorities(current, next))
    setPreview(undefined)
    setStatus('')
    setError(undefined)
    setHover([])
    setDragging('')
  }
  function dropProps(target: string, position: 'before' | 'same' | 'after') {
    return {
      onDragOver: (event: DragEvent) => {
        if (!dragMode || !dragging || dragging === target || busy) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        setHover(movePriority(current, dragging, target, position))
      },
      onDrop: (event: DragEvent) => {
        event.preventDefault()
        if (
          !dragMode ||
          !dragging ||
          busy ||
          event.dataTransfer.getData('text/plain') !== dragging
        )
          return
        place(dragging, target, position)
      },
    }
  }
  async function prepare() {
    if (!scope || !catalog || !route) return
    setBusy(true)
    setError(undefined)
    try {
      setPreview(
        await priorityPreview(scope, catalog.expected, route.route_id, changes)
      )
      setOperation(operationId())
    } catch (error) {
      setError(error)
    } finally {
      setBusy(false)
    }
  }
  async function undo() {
    if (!scope || !catalog || !lastOperation) return
    setBusy(true)
    setError(undefined)
    try {
      setPreview(await undoPreview(scope, catalog.expected, lastOperation))
      setOperation(operationId())
    } catch (error) {
      setError(error)
    } finally {
      setBusy(false)
    }
  }
  async function confirm() {
    if (!scope || !preview) return
    setBusy(true)
    setError(undefined)
    try {
      const result = await publish(scope, preview, operation)
      setLastOperation(operation)
      setPreview(undefined)
      setDraft(undefined)
      setStatus(
        result.applied_on_this_node
          ? 'Channel priorities published successfully.'
          : 'Published successfully. Requests will resume when the new configuration is ready.'
      )
      await reload()
    } catch (error) {
      setError(error)
      setStatus(
        'Publication could not be confirmed. Retry to check the same operation.'
      )
    } finally {
      setBusy(false)
    }
  }
  const positions = [
    ['before', 'Before this tier'],
    ['same', 'Same priority'],
    ['after', 'After this tier'],
  ] as const
  const rows = ordered.map((target) => {
    const pending = projected.find(
      (item) => item.target_id === target.target_id
    )?.priority
    const props = {
      draggable: dragMode && !busy,
      onDragStart: (event: DragEvent) => {
        if (!dragMode) return
        event.dataTransfer.setData('text/plain', target.target_id)
        event.dataTransfer.effectAllowed = 'move'
        setDragging(target.target_id)
      },
      onDragEnd: () => {
        setDragging('')
        setHover([])
      },
    }
    const identity = (
      <div>
        <p className='text-sm font-medium'>
          {t('Target')} #{target.target_id} · {t('Upstream')} #
          {target.upstream_id}
        </p>
        <p className='text-muted-foreground text-xs'>
          {target.model} · {t('Weight')}: {target.weight} ·{' '}
          {t(target.enabled ? 'Enabled' : 'Disabled')}
        </p>
      </div>
    )
    const priority = (
      <span className='font-mono text-sm'>
        {target.priority}
        {pending !== target.priority && (
          <strong className='text-primary'> → {pending}</strong>
        )}
      </span>
    )
    const destinations = dragMode && (
      <div className='flex flex-wrap gap-1'>
        {positions.map(([position, label]) => (
          <Button
            key={position}
            size='sm'
            variant='outline'
            disabled={busy || dragging === target.target_id}
            {...dropProps(target.target_id, position)}
            onClick={() => {
              if (moved && moved !== target.target_id)
                place(moved, target.target_id, position)
            }}
          >
            {t(label)}
          </Button>
        ))}
      </div>
    )
    return cards ? (
      <div
        key={target.target_id}
        role='listitem'
        {...props}
        className={`space-y-3 rounded-xl border p-3 ${dragMode ? 'cursor-grab active:cursor-grabbing' : ''}`}
      >
        {identity}
        <p>
          {t('Priority')}: {priority}
        </p>
        {destinations}
      </div>
    ) : (
      <TableRow
        key={target.target_id}
        {...props}
        className={dragMode ? 'cursor-grab active:cursor-grabbing' : ''}
      >
        <TableCell>{identity}</TableCell>
        <TableCell>{priority}</TableCell>
        <TableCell>{destinations}</TableCell>
      </TableRow>
    )
  })
  return (
    <section className='space-y-4' aria-label={t('Native channel priorities')}>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Smaller priorities run first. Targets with equal priority compete using their configured weights.'
        )}
      </p>
      {Boolean(loadError) && (
        <p role='alert' className='text-destructive text-sm'>
          {t(errorMessage(loadError))}
        </p>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button
          variant='outline'
          disabled={loading || busy}
          onClick={() => void reload()}
        >
          {loading ? t('Loading...') : t('Reload')}
        </Button>
        <Button
          variant='outline'
          disabled={busy || !lastOperation}
          onClick={() => void undo()}
        >
          {t('Undo last published order')}
        </Button>
      </div>
      {catalog && !catalog.can_edit_priorities && (
        <p role='status'>
          {t('You do not have permission to publish this change.')}
        </p>
      )}
      {catalog?.can_edit_priorities && route && (
        <>
          <NativeSelect
            aria-label={t('Model route')}
            value={route.route_id}
            disabled={busy}
            onChange={(e) => setRouteId(e.target.value)}
          >
            {catalog.routes.map((route) => (
              <NativeSelectOption key={route.route_id} value={route.route_id}>
                {route.model} · {route.group}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <div className='flex flex-wrap items-center gap-4'>
            <label className='flex items-center gap-2 text-sm'>
              <Switch
                checked={dragMode}
                disabled={busy}
                onCheckedChange={(value) => {
                  setDragMode(value)
                  setDragging('')
                  setHover([])
                }}
              />
              {t('Drag to reorder')}
            </label>
            <label className='flex items-center gap-2 text-sm'>
              <Switch checked={cards} onCheckedChange={setCards} />
              {t('Card view')}
            </label>
            <Button
              variant='ghost'
              disabled={busy || changes.length === 0}
              onClick={() => {
                setDraft(undefined)
                setPreview(undefined)
                setHover([])
              }}
            >
              {t('Discard changes')}
            </Button>
          </div>
          {cards ? (
            <div
              className='grid gap-3 sm:grid-cols-2'
              role='list'
              aria-label={t('Channel priority order')}
            >
              {rows}
            </div>
          ) : (
            <Table aria-label={t('Channel priority order')}>
              <TableHeader>
                <TableRow>
                  <TableHead>{t('Target')}</TableHead>
                  <TableHead>{t('Priority')}</TableHead>
                  <TableHead>{t('Destination target')}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>{rows}</TableBody>
            </Table>
          )}
          <p className='text-sm' aria-live='polite'>
            {hover.length > 0
              ? t('{{count}} priorities will change', { count: hover.length })
              : t('{{count}} priorities changed', { count: changes.length })}
          </p>
          <fieldset className='space-y-2 rounded-xl border p-3' disabled={busy}>
            <legend className='px-1 text-sm font-medium'>
              {t('Move with keyboard or touch')}
            </legend>
            <div className='flex flex-wrap items-center gap-2'>
              <NativeSelect
                aria-label={t('Target to move')}
                value={moved}
                onChange={(e) => setMoved(e.target.value)}
              >
                <NativeSelectOption value=''>
                  {t('Target to move')}
                </NativeSelectOption>
                {current.map((target) => (
                  <NativeSelectOption
                    key={target.target_id}
                    value={target.target_id}
                  >
                    #{target.target_id} · {target.model}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
              <NativeSelect
                aria-label={t('Destination target')}
                value={anchor}
                onChange={(e) => setAnchor(e.target.value)}
              >
                <NativeSelectOption value=''>
                  {t('Destination target')}
                </NativeSelectOption>
                {current
                  .filter((target) => target.target_id !== moved)
                  .map((target) => (
                    <NativeSelectOption
                      key={target.target_id}
                      value={target.target_id}
                    >
                      #{target.target_id} · {target.model}
                    </NativeSelectOption>
                  ))}
              </NativeSelect>
              {positions.map(([position, label]) => (
                <Button
                  key={position}
                  variant='outline'
                  size='sm'
                  disabled={!moved || !anchor || moved === anchor}
                  onClick={() => place(moved, anchor, position)}
                >
                  {t(label)}
                </Button>
              ))}
            </div>
          </fieldset>
          <Button
            disabled={busy || changes.length === 0}
            onClick={() => void prepare()}
          >
            {t('Preview priority changes')}
          </Button>
        </>
      )}
      {Boolean(error) && (
        <p role='alert' className='text-destructive text-sm'>
          {t(errorMessage(error))}
        </p>
      )}
      {status && (
        <p role='status' className='text-sm'>
          {t(status)}
        </p>
      )}
      {preview && (
        <PublicationReview
          preview={preview}
          busy={busy}
          onConfirm={() => void confirm()}
          onCancel={() => setPreview(undefined)}
        />
      )}
    </section>
  )
}
