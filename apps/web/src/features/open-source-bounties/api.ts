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
import axios from 'axios'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  activateBountyAccount,
  beginBountyOperation,
  bountyOperationsSnapshot,
  completeBountyOperation,
  hasBountyOperation,
  pendingBountyOperation,
  updateBountyOperation,
  waitForBountyOperation,
} from './native-operation'
import type {
  BountyId,
  BountyNotification,
  BountyChallenge,
  BountyDraftInput,
  BountyDispute,
  BountyDisputeReason,
  BountyFeeConfig,
  BountyProject,
  BountyProjectDetail,
  BountyTipNotification,
} from './types'

class BountyActionRejected extends Error {
  constructor(
    message: string,
    readonly code?: string
  ) {
    super(message)
  }
}

interface ApiEnvelope<T> {
  success: boolean
  code?: string
  message?: string
  data: T
}

async function unwrap<T>(
  request: Promise<{ data: ApiEnvelope<T>; status?: number }>,
  expectedOperationID?: string
) {
  const response = await request
  if (!response.data.success) {
    throw new BountyActionRejected(
      response.data.message || 'Open-source bounty request failed',
      response.data.code
    )
  }
  if (response.status === 202) {
    const id = pendingBountyOperation(response.data.data, expectedOperationID)
    await waitForBountyOperation(id)
    return unwrap<T>(
      api.get(
        `/api/open-source-bounties/operations/${encodeURIComponent(id)}/result`
      )
    )
  }
  return response.data.data
}
async function nativeMutation<T>(
  method: 'post' | 'delete',
  url: string,
  data?: unknown,
  idempotencyKey?: string
): Promise<T> {
  const user = useAuthStore.getState().auth.user
  activateBountyAccount(user ? String(user.id) : 'session')
  const id = idempotencyKey ?? crypto.randomUUID()
  if (!beginBountyOperation(id)) {
    await waitForBountyOperation(id)
    return unwrap<T>(
      api.get(
        `/api/open-source-bounties/operations/${encodeURIComponent(id)}/result`
      )
    )
  }
  try {
    const config = {
      headers: {
        'X-Lmm-Operation-ID': id,
        'Idempotency-Key': idempotencyKey ?? id,
      },
      skipErrorHandler: true,
      skipBusinessError: true,
    }
    const result = await unwrap<T>(
      method === 'post'
        ? api.post<ApiEnvelope<T>>(url, data, config)
        : api.delete<ApiEnvelope<T>>(url, config),
      id
    )
    completeBountyOperation(id)
    return result
  } catch (error) {
    if (
      bountyOperationsSnapshot().find((operation) => operation.id === id)
        ?.phase === 'preparing' &&
      (error instanceof BountyActionRejected ||
        (axios.isAxiosError(error) &&
          [400, 401, 403, 404, 409].includes(error.response?.status ?? 0)))
    ) {
      completeBountyOperation(id)
      throw error
    }
    if (
      !hasBountyOperation(id) ||
      (error instanceof Error && error.message === 'Session changed')
    ) {
      throw error
    }
    updateBountyOperation(id, { phase: 'unknown' })
    await waitForBountyOperation(id)
    const result = await unwrap<T>(
      api.get(
        `/api/open-source-bounties/operations/${encodeURIComponent(id)}/result`
      )
    )
    completeBountyOperation(id)
    return result
  }
}

export async function listBounties() {
  const result = await unwrap<{
    items: BountyProject[] | null
    total: number
    page: number
    page_size: number
  }>(
    api.get('/api/open-source-bounties?page=1&page_size=50', {
      // Public challenges are optional on deployments that have not mounted
      // the bounty candidate yet.  ChallengeList owns the inline fallback;
      // never surface a probe 401/404 as a global toast.
      skipBusinessError: true,
      skipErrorHandler: true,
    })
  )
  return { ...result, items: result.items ?? [] }
}

export function getBountyConfig() {
  return unwrap<BountyFeeConfig>(api.get('/api/open-source-bounties/config'))
}

export function listOwnedBounties(archived = false) {
  return unwrap<BountyProject[]>(
    api.get(
      `/api/open-source-bounties/mine?archived=${archived ? 'true' : 'false'}`
    )
  )
}

export async function getPendingBountyReviewCount() {
  const result = await unwrap<{ total: number }>(
    api.get('/api/todos?category=open_source_bounty_review&p=1&page_size=1', {
      skipBusinessError: true,
      skipErrorHandler: true,
    })
  )
  return Number.isSafeInteger(result.total) && result.total > 0
    ? result.total
    : 0
}

export function listAcceptedBounties() {
  return unwrap<BountyChallenge[]>(
    api.get('/api/open-source-bounties/accepted')
  )
}

export function getBountyDetail(projectId: BountyId) {
  return unwrap<BountyProjectDetail>(
    api.get(
      `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}`
    )
  )
}

export function createBounty(input: BountyDraftInput) {
  return unwrap<BountyProject>(api.post('/api/open-source-bounties', input))
}

export function updateBounty(projectId: BountyId, input: BountyDraftInput) {
  return unwrap<BountyProject>(
    api.put(
      `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}`,
      input
    )
  )
}

export function deleteBounty(projectId: BountyId) {
  return nativeMutation<null>(
    'delete',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}`
  )
}

export function publishBounty(projectId: BountyId) {
  return nativeMutation<{ project: BountyProject; charged_quota: number }>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/publish`
  )
}
export function pauseBounty(projectId: BountyId) {
  return nativeMutation<BountyProject>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/pause`
  )
}
export function resumeBounty(projectId: BountyId) {
  return nativeMutation<BountyProject>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/resume`
  )
}
export function closeBounty(projectId: BountyId) {
  return nativeMutation<{ project: BountyProject; refunded_quota: number }>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/close`
  )
}
export function archiveBounty(projectId: BountyId) {
  return unwrap<BountyProject>(
    api.post(
      `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/archive`
    )
  )
}

export function unarchiveBounty(projectId: BountyId) {
  return unwrap<BountyProject>(
    api.post(
      `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/unarchive`
    )
  )
}

export function acceptBounty(projectId: BountyId, githubHandle: string) {
  return nativeMutation<BountyChallenge>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/accept`,
    {
      github_handle: githubHandle,
    }
  )
}
export function submitChallenge(
  projectId: BountyId,
  input: {
    issue_url: string
    pull_request_url: string
    submission_note: string
  }
) {
  return nativeMutation<BountyChallenge>(
    'post',
    `/api/open-source-bounties/projects/${encodeURIComponent(projectId)}/submit`,
    input
  )
}
export function withdrawChallenge(challengeId: BountyId) {
  return nativeMutation<BountyChallenge>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/withdraw`
  )
}
export function cancelChallenge(challengeId: BountyId) {
  return nativeMutation<BountyChallenge>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/cancel`
  )
}
export function reviewChallenge(
  challengeId: BountyId,
  action: 'approve' | 'reject',
  input: {
    review_note: string
    rating_score: number
    rating_comment: string
  }
) {
  return nativeMutation<{
    challenge: BountyChallenge
    transferred_quota: number
  }>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/${action}`,
    input
  )
}
export function tipChallenge(
  challengeId: BountyId,
  input: { quota: number; note: string },
  idempotencyKey: string
) {
  return nativeMutation<{
    challenge: BountyChallenge
    transferred_quota: number
    remaining_quota: number
  }>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/tip`,
    input,
    idempotencyKey
  )
}
export function listBountyNotifications() {
  return unwrap<BountyNotification[]>(
    api.get('/api/open-source-bounties/notifications', {
      // Older production candidates may advertise the capability before the
      // unified route is mounted. A missing optional notification feed must
      // not become a global "Not Found" toast or block the bounty page.
      skipBusinessError: true,
      skipErrorHandler: true,
    })
  )
}

export function markBountyNotificationsRead() {
  return unwrap<null>(
    api.post('/api/open-source-bounties/notifications/read', undefined, {
      skipBusinessError: true,
      skipErrorHandler: true,
    })
  )
}

export function listReceivedBountyTips() {
  return unwrap<BountyTipNotification[]>(
    api.get('/api/open-source-bounties/tips/received')
  )
}

export function markReceivedBountyTipsRead() {
  return unwrap<null>(api.post('/api/open-source-bounties/tips/received/read'))
}

export async function listCompatibleBountyNotifications(
  supportsUnifiedNotifications: boolean
): Promise<BountyNotification[]> {
  if (supportsUnifiedNotifications) return listBountyNotifications()

  const tips = await listReceivedBountyTips()
  return tips.map((tip) => ({ ...tip, kind: 'tip_transfer' }))
}

export function markCompatibleBountyNotificationsRead(
  supportsUnifiedNotifications: boolean
) {
  return supportsUnifiedNotifications
    ? markBountyNotificationsRead()
    : markReceivedBountyTipsRead()
}

export function thankBountyTip(tipId: BountyId) {
  return unwrap<BountyTipNotification>(
    api.post(
      `/api/open-source-bounties/tips/${encodeURIComponent(tipId)}/thank`
    )
  )
}

export function rateBountyOwner(
  challengeId: BountyId,
  input: { score: number; comment: string }
) {
  return nativeMutation<BountyChallenge>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/rate-owner`,
    input
  )
}
export function openBountyDispute(
  challengeId: BountyId,
  input: { reason: BountyDisputeReason; statement: string }
) {
  return nativeMutation<BountyDispute>(
    'post',
    `/api/open-source-bounties/challenges/${encodeURIComponent(challengeId)}/disputes`,
    input
  )
}
export function listMyBountyDisputes() {
  return unwrap<BountyDispute[]>(
    api.get('/api/open-source-bounties/disputes/mine')
  )
}

export function listAdminBountyDisputes() {
  return unwrap<BountyDispute[]>(
    api.get('/api/open-source-bounties/disputes/admin')
  )
}

export function resolveBountyDispute(
  disputeId: BountyId,
  input: { action: 'pay' | 'deny'; resolution: string }
) {
  return nativeMutation<{ dispute: BountyDispute; transferred_quota: number }>(
    'post',
    `/api/open-source-bounties/disputes/${encodeURIComponent(disputeId)}/resolve`,
    input
  )
}
