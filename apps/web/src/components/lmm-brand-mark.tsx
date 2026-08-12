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

/** An abstract woven mark for the forge: two paths crossing into one system. */
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
        x='5'
        y='5'
        width='46'
        height='46'
        rx='16'
        fill='var(--forge-brand-mark-surface)'
        stroke='var(--forge-brand-mark-ink)'
        strokeWidth='2'
      />
      <path
        d='M13 34c5-10 10-15 16-15 5 0 10 4 14 12'
        fill='none'
        stroke='var(--forge-brand-mark-ink)'
        strokeWidth='4.5'
        strokeLinecap='round'
        strokeLinejoin='round'
      />
      <path
        d='M13 22c5 10 10 15 16 15 5 0 10-4 14-12'
        fill='none'
        stroke='var(--forge-brand-mark-accent)'
        strokeWidth='4.5'
        strokeLinecap='round'
        strokeLinejoin='round'
      />
      <circle cx='13' cy='28' r='3' fill='var(--forge-brand-mark-ink)' />
    </svg>
  )
}
