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
import { useTranslation } from 'react-i18next'

import { useTopNavLinks } from '@/hooks/use-top-nav-links'
import { cn } from '@/lib/utils'

import { defaultTopNavLinks } from '../config/top-nav.config'
import type { TopNavLink } from '../types'
import { getNavLinkKey } from './nav-link-key'

interface PublicNavigationProps {
  /**
   * Custom navigation links
   * If not provided, will use dynamic links from backend or defaults
   */
  links?: TopNavLink[]
  /**
   * Additional className
   */
  className?: string
}

/**
 * Public navigation component that matches Launch UI template styling
 * Used in PublicHeader for desktop navigation
 */
export function PublicNavigation({
  links: providedLinks,
  className,
}: PublicNavigationProps = {}) {
  // Use the same logic as AppHeader: prioritize dynamic links from backend
  const dynamicLinks = useTopNavLinks()
  const { t } = useTranslation()
  const pathname = useRouterState().location.pathname
  const defaultLinks = providedLinks || defaultTopNavLinks
  const links = dynamicLinks.length > 0 ? dynamicLinks : defaultLinks

  return (
    <nav
      aria-label={t('Header navigation')}
      className={cn('hidden items-center gap-1 md:flex', className)}
    >
      {links.map((link) => {
        const isActive = pathname === link.href
        const linkClassName = cn(
          'text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:ring-ring focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-none touch-manipulation inline-flex h-9 w-max items-center justify-center rounded-md bg-transparent px-4 py-2 text-sm font-medium transition-colors',
          link.disabled && 'pointer-events-none opacity-50'
        )
        const handleClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
          if (link.disabled) event.preventDefault()
        }

        // Handle external links
        if (link.external) {
          return (
            <a
              key={getNavLinkKey(link)}
              href={link.href}
              target='_blank'
              rel='noopener noreferrer'
              aria-current={isActive ? 'page' : undefined}
              aria-disabled={link.disabled || undefined}
              tabIndex={link.disabled ? -1 : undefined}
              onClick={handleClick}
              className={linkClassName}
            >
              {link.title}
            </a>
          )
        }
        // Handle internal links
        return (
          <Link
            key={getNavLinkKey(link)}
            to={link.href}
            disabled={link.disabled}
            aria-current={isActive ? 'page' : undefined}
            aria-disabled={link.disabled || undefined}
            tabIndex={link.disabled ? -1 : undefined}
            onClick={handleClick}
            className={linkClassName}
          >
            {link.title}
          </Link>
        )
      })}
    </nav>
  )
}
