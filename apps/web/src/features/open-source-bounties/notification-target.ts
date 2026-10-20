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
import {
  Award01Icon,
  GiftIcon,
  JusticeScale01Icon,
} from '@hugeicons/core-free-icons'
import type { TFunction } from 'i18next'

import type { BountyId, BountyNotification, BountyProjectDetail } from './types'

export interface BountyNotificationDetailTarget {
  projectId: BountyId
  challengeId: BountyId
}

export function selectBountyNotificationChallenge(
  detail: BountyProjectDetail,
  target: BountyNotificationDetailTarget
) {
  if (detail.project.id !== target.projectId) return null
  return (
    detail.challenges.find(
      (challenge) =>
        challenge.id === target.challengeId &&
        challenge.project_id === target.projectId
    ) ?? null
  )
}

export function bountyNotificationPresentation(
  item: BountyNotification,
  t: TFunction
) {
  if (item.kind === 'tip_transfer') {
    return {
      icon: GiftIcon,
      message: t('sent you a tip of'),
      showNoteAndThank: true,
    }
  }
  if (item.kind === 'dispute_reward_transfer') {
    return {
      icon: JusticeScale01Icon,
      message: t('paid your bounty reward after dispute resolution'),
      showNoteAndThank: false,
    }
  }
  return {
    icon: Award01Icon,
    message: t('approved and paid your bounty reward'),
    showNoteAndThank: false,
  }
}

export function bountyNotificationSearch(item: BountyNotification) {
  return { projectId: item.project_id, challengeId: item.challenge_id }
}
