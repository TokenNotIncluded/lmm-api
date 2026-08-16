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
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  getDisabledOAuthRegistrationMethods,
  hasOAuthRegistrationProvider,
  hasRegistrationMethod,
  isPasswordRegistrationEnabled,
  isRegistrationEnabled,
} from './registration'

describe('registration availability', () => {
  test('requires the global registration switch', () => {
    const status = {
      register_enabled: false,
      password_register_enabled: true,
      github_oauth: true,
    }

    assert.equal(isRegistrationEnabled(status), false)
    assert.equal(isPasswordRegistrationEnabled(status), false)
    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(hasRegistrationMethod(status), false)
  })

  test('hides password registration without hiding OAuth registration', () => {
    const status = {
      register_enabled: true,
      password_register_enabled: false,
      github_oauth: true,
    }

    assert.equal(isPasswordRegistrationEnabled(status), false)
    assert.equal(hasOAuthRegistrationProvider(status), true)
    assert.equal(hasRegistrationMethod(status), true)
  })

  test('filters disabled OAuth channels, including custom providers', () => {
    const status = {
      github_oauth: true,
      custom_oauth_providers: [
        {
          id: 1,
          name: 'Company SSO',
          slug: 'company-sso',
          icon: '',
          client_id: 'client',
          authorization_endpoint: 'https://sso.example.test/authorize',
          scopes: '',
        },
      ],
      oauth_registration_disabled_methods: ['GITHUB', 'custom:company-sso'],
    }

    assert.deepEqual([...getDisabledOAuthRegistrationMethods(status)].sort(), [
      'custom:company-sso',
      'github',
    ])
    assert.equal(hasOAuthRegistrationProvider(status), false)
  })

  test('does not block existing login when OAuth registration is disabled', () => {
    const status = {
      github_oauth: true,
      oauth_registration_disabled_methods: ['github'],
    }

    // The helper only answers the sign-up question. Sign-in renders its own
    // OAuthProviders instance without registrationOnly, so this setting does
    // not affect the login path.
    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(isRegistrationEnabled(status), true)
  })
})
