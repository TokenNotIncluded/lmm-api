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
/*
Copyright (C) 2026 LIghtJUNction
*/
import type { L1OnboardingStepId, L1OnboardingTodo } from './api'

const STEP_IDS: L1OnboardingStepId[] = [
  'create_api_key',
  'install_client',
  'configure_client',
  'first_successful_response',
]

export function getAssistantOnboardingTodoSteps(
  todo: Pick<L1OnboardingTodo, 'steps'>
) {
  const completed = new Set(
    todo.steps
      .filter((step) => step.status === 'completed')
      .map((step) => step.id)
  )

  return STEP_IDS.map((id, index) => ({
    id,
    complete: completed.has(id),
    available:
      index === 0 || completed.has(STEP_IDS[index - 1] as L1OnboardingStepId),
  }))
}
