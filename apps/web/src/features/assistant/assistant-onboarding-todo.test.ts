import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { getAssistantOnboardingTodoSteps } from './assistant-onboarding-todo-state'

describe('assistant onboarding todo', () => {
  test('keeps the four L1 steps ordered and gated by server progress', () => {
    const steps = getAssistantOnboardingTodoSteps({
      steps: [
        { id: 'create_api_key', status: 'pending' },
        { id: 'install_client', status: 'pending' },
        { id: 'configure_client', status: 'pending' },
        { id: 'first_successful_response', status: 'pending' },
      ],
    })

    assert.deepEqual(
      steps.map((step) => [step.id, step.complete, step.available]),
      [
        ['create_api_key', false, true],
        ['install_client', false, false],
        ['configure_client', false, false],
        ['first_successful_response', false, false],
      ]
    )
  })

  test('does not infer installation or configuration from key creation', () => {
    const steps = getAssistantOnboardingTodoSteps({
      steps: [
        { id: 'create_api_key', status: 'completed' },
        { id: 'install_client', status: 'pending' },
        { id: 'configure_client', status: 'pending' },
        { id: 'first_successful_response', status: 'pending' },
      ],
    })

    assert.equal(steps[0]?.complete, true)
    assert.equal(steps[1]?.complete, false)
    assert.equal(steps[1]?.available, true)
    assert.equal(steps[2]?.available, false)
  })

  test('only a server-confirmed successful response completes the final state', () => {
    const steps = getAssistantOnboardingTodoSteps({
      steps: [
        { id: 'create_api_key', status: 'completed' },
        { id: 'install_client', status: 'completed' },
        { id: 'configure_client', status: 'completed' },
        { id: 'first_successful_response', status: 'completed' },
      ],
    })

    assert.equal(
      steps.every((step) => step.complete),
      true
    )
  })
})
