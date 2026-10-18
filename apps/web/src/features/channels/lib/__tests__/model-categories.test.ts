/* Copyright (C) 2026 LMM API contributors. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { categorizeModels, getModelCategory } from '../model-categories'

test('recognizes NVIDIA and Cloudflare model families at name boundaries', () => {
  for (const model of [
    'nv-embed-v2',
    'nv-rerank-qa-mistral-4b',
    'riva-translate-4b',
    'parakeet-tdt-0.6b-v3',
    'canary-1b',
    'nemoretriever-parse',
    'nvidia/llama-3.1-nemotron-70b-instruct',
    ' NVIDIA/NV-EMBED-V2 ',
  ]) {
    assert.equal(getModelCategory(model), 'NVIDIA', model)
  }
  for (const model of ['clef', 'clef-v1', 'cloudflare/clef:free']) {
    assert.equal(getModelCategory(model), 'Cloudflare', model)
  }
  for (const model of ['private-riva-model', 'proclef', 'canarybird']) {
    assert.equal(getModelCategory(model), 'Other', model)
  }
})

test('inkling belongs to Thinking Machines while Kling keeps its own names', () => {
  const models = [
    'thinkingmachines/inkling',
    'thinkingmachines/inkling-small',
    'thinkingmachines/inkling:free',
    'thinkingmachines/inkling-small:free',
    'inkling',
  ]
  assert.deepEqual(categorizeModels(models), { 'Thinking Machines': models })
  for (const model of ['kling', 'kling-v2', 'kling/v2.1', 'kling.v2']) {
    assert.equal(getModelCategory(model), 'Kling', model)
  }
  assert.equal(getModelCategory('sparkling-model'), 'Other')
  assert.equal(getModelCategory('meta-llama/llama-3.1-8b-instruct'), 'Meta')
  assert.equal(getModelCategory('mistralai/mixtral-8x7b-instruct'), 'Mistral')
})
