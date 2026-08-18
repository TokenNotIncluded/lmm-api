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
import type { SVGProps } from 'react'

import { cn } from '@/lib/utils'

export const LMM_BRAND_NAME = 'LMM Forge'

type LmmBrandMarkProps = SVGProps<SVGSVGElement> & {
  title?: string
}

/** A compact angular monogram: an M-shaped forge frame over a hot base. */
export function LmmBrandMark({
  className,
  title,
  ...props
}: LmmBrandMarkProps) {
  return (
    <svg
      viewBox='0 0 56 56'
      xmlns='http://www.w3.org/2000/svg'
      role={title ? 'img' : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable='false'
      className={cn('shrink-0', className)}
      {...props}
    >
      <rect
        x='4'
        y='4'
        width='48'
        height='48'
        rx='14'
        fill='var(--forge-brand-mark-surface)'
      />
      <path
        d='M15 38V18l13 13 13-13v20'
        fill='none'
        stroke='var(--forge-brand-mark-ink)'
        strokeWidth='4.25'
        strokeLinecap='round'
        strokeLinejoin='round'
      />
      <path
        d='M16 41h24'
        fill='none'
        stroke='var(--forge-brand-mark-accent)'
        strokeWidth='3.5'
        strokeLinecap='round'
      />
      <rect
        x='4.75'
        y='4.75'
        width='46.5'
        height='46.5'
        rx='13.25'
        fill='none'
        stroke='var(--forge-brand-mark-ink)'
        strokeOpacity='0.16'
        strokeWidth='1.5'
      />
    </svg>
  )
}
