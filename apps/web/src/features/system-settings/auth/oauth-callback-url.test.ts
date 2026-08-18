/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  buildOAuthCallbackUrl,
  normalizeServerAddress,
  resolveOAuthSiteUrl,
} from './oauth-callback-url'

describe('server address normalization', () => {
  test('adds https to a bare host', () => {
    assert.equal(
      normalizeServerAddress(' api.lmm.best/ '),
      'https://api.lmm.best'
    )
  })

  test('preserves an explicit scheme and removes trailing slashes', () => {
    assert.equal(
      normalizeServerAddress('http://localhost:3000///'),
      'http://localhost:3000'
    )
  })

  test('uses the fallback only when the setting is empty', () => {
    assert.equal(
      resolveOAuthSiteUrl('', 'https://fallback.example'),
      'https://fallback.example'
    )
    assert.equal(
      resolveOAuthSiteUrl('api.lmm.best', 'https://fallback.example'),
      'https://api.lmm.best'
    )
  })

  test('builds callback URLs from normalized addresses', () => {
    assert.equal(
      buildOAuthCallbackUrl(
        'api.lmm.best',
        '/github',
        'https://fallback.example'
      ),
      'https://api.lmm.best/oauth/github'
    )
  })
})
