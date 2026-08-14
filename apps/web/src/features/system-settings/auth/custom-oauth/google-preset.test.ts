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
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { OAUTH_PRESETS } from './types'

describe('Google custom OAuth preset', () => {
  test('provides the standard OpenID Connect endpoints without a base URL', () => {
    const google = OAUTH_PRESETS.find((preset) => preset.key === 'google')
    assert.deepEqual(google, {
      key: 'google',
      name: 'Google',
      icon: 'google',
      authorization_endpoint: 'https://accounts.google.com/o/oauth2/v2/auth',
      token_endpoint: 'https://oauth2.googleapis.com/token',
      user_info_endpoint: 'https://openidconnect.googleapis.com/v1/userinfo',
      scopes: 'openid profile email',
      user_id_field: 'sub',
      username_field: 'email',
      display_name_field: 'name',
      email_field: 'email',
      needsBaseUrl: false,
    })
  })
})
