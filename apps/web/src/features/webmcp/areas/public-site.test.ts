/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { publicSiteTools } from './public-site'

test('public developer summary lists distinct public, Session and OAuth catalog contracts', async () => {
  const tools = publicSiteTools({
    router: {
      navigate: async () => undefined,
      subscribe: () => () => undefined,
    },
  })
  const developer = tools.find((tool) => tool.name === 'lmm_developers_read')
  assert.ok(developer)
  const result = await developer.execute(
    {},
    { signal: new AbortController().signal }
  )
  const content = JSON.stringify(result)
  assert.match(content, /\/core\/v1\/pricing\/public/)
  assert.match(content, /\/core\/v1\/pricing\/session/)
  assert.match(content, /\/api\/oauth2\/catalog/)
  assert.doesNotMatch(content, /"path":"\/api\/pricing"|DSH/)
})
