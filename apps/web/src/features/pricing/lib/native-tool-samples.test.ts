/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildNativeToolSample } from './native-tool-samples'

test('native tool samples carry the route model and the correct operation payload', async () => {
  for (const endpointType of ['deepseek-search', 'deepseek-fetch'] as const) {
    const calls: { url: string; init: RequestInit }[] = []
    const sample = buildNativeToolSample('javascript', {
      baseUrl: 'https://gateway.example',
      apiKeyEnv: 'LMM_API_KEY',
      modelName: 'local-tool',
      endpointType,
      endpointPath:
        endpointType === 'deepseek-search'
          ? '/v1/tools/search'
          : '/v1/tools/fetch',
    })
    const run = new Function(
      'fetch',
      'process',
      'console',
      `return (async () => {${sample}})()`
    )
    await run(
      async (url: string, init: RequestInit) => {
        calls.push({ url, init })
        return { ok: true, json: async () => ({}) }
      },
      { env: { LMM_API_KEY: 'test-key' } },
      { log() {} }
    )
    assert.equal(calls.length, 1)
    const { url, init } = calls[0]!
    assert.ok(
      url.endsWith(
        endpointType === 'deepseek-search'
          ? '/v1/tools/search'
          : '/v1/tools/fetch'
      )
    )
    assert.deepEqual(
      JSON.parse(String(init.body)),
      endpointType === 'deepseek-search'
        ? { model: 'local-tool', query: 'quantum entanglement' }
        : { model: 'local-tool', url: 'https://example.com/' }
    )
    assert.equal(
      (init.headers as Record<string, string>).Authorization,
      'Bearer test-key'
    )
  }
})

test('tool samples quote model and URL text and never interpolate an invalid key environment', () => {
  const context = {
    baseUrl: "https://example.test/a'b",
    endpointPath: '/v1/tools/search',
    apiKeyEnv: 'BAD;execute()',
    modelName: 'm\'\\"\n',
    endpointType: 'deepseek-search' as const,
  }
  for (const language of [
    'curl',
    'python',
    'typescript',
    'javascript',
  ] as const) {
    const sample = buildNativeToolSample(language, context)
    assert.ok(!sample.includes('BAD;execute()'))
    assert.ok(sample.includes('LMM_API_KEY'))
    assert.ok(!sample.includes('messages'))
  }
  assert.ok(buildNativeToolSample('curl', context).includes("'\\''"))
})
