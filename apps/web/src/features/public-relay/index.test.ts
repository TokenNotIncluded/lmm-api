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
  test('renders the complete channel drawer outside the slot-only page layout', () => {
    const layoutEnd = source.indexOf('</SectionPageLayout>')
    const submitDrawer = source.indexOf('<ChannelMutateDrawer')

    assert.notEqual(layoutEnd, -1)
    assert.notEqual(submitDrawer, -1)
    assert.ok(
      layoutEnd < submitDrawer,
      'the page layout must close before its controlled drawer is rendered'
    )
    assert.match(source, /<ChannelsProvider>/)
    assert.match(source, /transformFormDataToCreatePayload/)
    assert.match(source, /fixedGroup: configQuery\.data\?\.group \?\? ''/)
    assert.match(
      source,
      /<Button type='button' onClick=\{\(\) => setSubmitOpen\(true\)\}>/
    )
  })

  test('does not render a failed public relay query as an empty channel list', () => {
    assert.match(
      source,
      /routingQuery\.isError[\s\S]*<PublicRelayLoadError[\s\S]*routingQuery\.refetch/
    )
    assert.match(
      source,
      /allQuery\.isError[\s\S]*<PublicRelayLoadError[\s\S]*allQuery\.refetch/
    )
    assert.match(
      source,
      /mineQuery\.isError[\s\S]*<PublicRelayLoadError[\s\S]*mineQuery\.refetch/
    )
  })
})
