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

export function IconGoogle(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox='0 0 18 18' aria-hidden='true' {...props}>
      <path
        fill='#4285F4'
        d='M17.64 9.205c0-.639-.057-1.252-.164-1.841H9v3.481h4.844a4.14 4.14 0 0 1-1.797 2.715v2.259h2.909c1.702-1.567 2.684-3.874 2.684-6.614Z'
      />
      <path
        fill='#34A853'
        d='M9 18c2.43 0 4.467-.806 5.956-2.181l-2.909-2.259c-.806.54-1.835.859-3.047.859-2.344 0-4.328-1.585-5.037-3.714H.956v2.332A9 9 0 0 0 9 18Z'
      />
      <path
        fill='#FBBC05'
        d='M3.963 10.705A5.413 5.413 0 0 1 3.681 9c0-.592.102-1.167.282-1.705V4.963H.956A9 9 0 0 0 0 9c0 1.45.347 2.824.956 4.037l3.007-2.332Z'
      />
      <path
        fill='#EA4335'
        d='M9 3.581c1.321 0 2.507.454 3.442 1.346l2.581-2.581C13.463.892 11.426 0 9 0A9 9 0 0 0 .956 4.963l3.007 2.332C4.672 5.166 6.656 3.581 9 3.581Z'
      />
    </svg>
  )
}
