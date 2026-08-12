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
