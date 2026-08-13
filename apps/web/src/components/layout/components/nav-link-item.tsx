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
import { Link, useRouterState } from '@tanstack/react-router'

import { cn } from '@/lib/utils'

import type { TopNavLink } from '../types'
import { getNavLinkKey } from './nav-link-key'

interface NavLinkItemProps {
  link: TopNavLink
  className?: string
}

/**
 * Renders a single navigation link (internal or external)
 * Handles routing and proper link attributes
 */
export function NavLinkItem({ link, className }: NavLinkItemProps) {
  const pathname = useRouterState().location.pathname
  const isActive = link.isActive ?? pathname === link.href
  const linkClassName = cn(
    'text-muted-foreground hover:text-foreground focus-visible:ring-ring focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-none touch-manipulation transition-colors',
    link.disabled && 'pointer-events-none opacity-50',
    className
  )
  const handleClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
    if (link.disabled) event.preventDefault()
  }

  if (link.external) {
    return (
      <a
        href={link.href}
        target='_blank'
        rel='noopener noreferrer'
        className={linkClassName}
        aria-current={isActive ? 'page' : undefined}
        aria-disabled={link.disabled || undefined}
        tabIndex={link.disabled ? -1 : undefined}
        onClick={handleClick}
      >
        {link.title}
      </a>
    )
  }

  return (
    <Link
      to={link.href}
      className={linkClassName}
      disabled={link.disabled}
      aria-current={isActive ? 'page' : undefined}
      aria-disabled={link.disabled || undefined}
      tabIndex={link.disabled ? -1 : undefined}
      onClick={handleClick}
    >
      {link.title}
    </Link>
  )
}

interface NavLinkListProps {
  links: TopNavLink[]
  className?: string
  itemClassName?: string
}

/**
 * Renders a list of navigation links
 * Used in both desktop and mobile navigation
 */
export function NavLinkList({
  links,
  className,
  itemClassName,
}: NavLinkListProps) {
  return links.map((link) => (
    <NavLinkItem
      key={getNavLinkKey(link)}
      link={link}
      className={cn(className, itemClassName)}
    />
  ))
}
