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
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { AccountActionRequestsPanel } from '@/features/users/components/account-action-requests-panel'
import { AssistantLeadsPanel } from '@/features/users/components/assistant-leads-panel'
import { DeveloperAccessRequestsPanel } from '@/features/users/components/developer-access-requests-panel'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { UnifiedTodoList } from './unified-todo-list'

export function Todos() {
  const { t } = useTranslation()
  const isAdmin = useAuthStore(
    (state) => (state.auth.user?.role ?? 0) >= ROLE.ADMIN
  )

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('To-dos')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        <div className='mx-auto flex min-h-0 w-full max-w-5xl flex-col gap-14 pb-20'>
          <UnifiedTodoList />
          {isAdmin ? (
            <>
              <AdminTodoSection title={t('Assistant support tasks')}>
                <AssistantLeadsPanel />
              </AdminTodoSection>
              <AdminTodoSection title={t('Account safety review')}>
                <AccountActionRequestsPanel />
              </AdminTodoSection>
              <AdminTodoSection title={t('L1 access requests')}>
                <DeveloperAccessRequestsPanel />
              </AdminTodoSection>
            </>
          ) : null}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}

function AdminTodoSection(props: { title: string; children: ReactNode }) {
  const [expanded, setExpanded] = useState(false)
  const [mounted, setMounted] = useState(false)

  return (
    <details
      className='border-border border-t py-5'
      open={expanded}
      onToggle={(event) => {
        const open = event.currentTarget.open
        setExpanded(open)
        if (open) setMounted(true)
      }}
    >
      <summary className='text-foreground cursor-pointer list-none text-sm font-medium [&::-webkit-details-marker]:hidden'>
        <span className='inline-flex items-center gap-2'>
          <span
            aria-hidden='true'
            className='text-muted-foreground inline-block text-xs'
          >
            ›
          </span>
          {props.title}
        </span>
      </summary>
      {mounted ? <div className='pt-5'>{props.children}</div> : null}
    </details>
  )
}
