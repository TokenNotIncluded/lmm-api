/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8')

describe('public relay sharing entry point', () => {
  test('renders dialog portals outside the slot-only page layout', () => {
    const layoutEnd = source.indexOf('</SectionPageLayout>')
    const submitDialog = source.indexOf('<Dialog open={submitOpen}')

    assert.notEqual(layoutEnd, -1)
    assert.notEqual(submitDialog, -1)
    assert.ok(
      layoutEnd < submitDialog,
      'the page layout must close before its controlled dialogs are rendered'
    )
    assert.match(
      source,
      /<Button type='button' onClick=\{\(\) => setSubmitOpen\(true\)\}>/
    )
  })
})
