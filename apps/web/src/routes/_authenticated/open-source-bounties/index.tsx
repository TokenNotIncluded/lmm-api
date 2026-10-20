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
import { createFileRoute } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'
import { z } from 'zod'

import { OpenSourceBounties } from '@/features/open-source-bounties'

const bountyId = z.union([
  z.number().int().positive(),
  z.string().regex(/^[A-Za-z0-9_.:-]{1,128}$/),
])

const bountySearchSchema = z.object({
  projectId: bountyId.optional(),
  challengeId: bountyId.optional(),
})

export const Route = createFileRoute('/_authenticated/open-source-bounties/')({
  validateSearch: bountySearchSchema,
  component: RouteComponent,
})

function RouteComponent() {
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const detailTarget = useMemo(
    () =>
      search.projectId && search.challengeId
        ? { projectId: search.projectId, challengeId: search.challengeId }
        : undefined,
    [search.challengeId, search.projectId]
  )
  const clearDetailTarget = useCallback(() => {
    void navigate({ search: {}, replace: true })
  }, [navigate])

  return (
    <OpenSourceBounties
      detailTarget={detailTarget}
      onDetailTargetConsumed={clearDetailTarget}
    />
  )
}
