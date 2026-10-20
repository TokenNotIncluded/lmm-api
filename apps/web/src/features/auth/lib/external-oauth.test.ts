/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  EXTERNAL_BROWSER_PRESETS,
  type ExternalBrowserProvider,
  buildExternalBrowserStartRequest,
  isExternalMFAContinuation,
  oauthCallbackSearchSchema,
  parseBrowserAuthorizationResponse,
  validateExternalBrowserAuthorizationUrl,
} from './external-oauth'

const providers: ExternalBrowserProvider[] = ['github', 'discord', 'linuxdo']
const origin = 'https://site.example.test'

function authorizationUrl(provider: ExternalBrowserProvider) {
  const preset = EXTERNAL_BROWSER_PRESETS[provider]
  const url = new URL(preset.authorization_endpoint)
  for (const [key, value] of Object.entries({
    client_id: 'synthetic-client',
    response_type: 'code',
    scope: preset.scope,
    state: 'server-owned-state',
    redirect_uri: `${origin}${preset.callbacks[0]}`,
  })) {
    url.searchParams.set(key, value)
  }
  if (preset.pkce) {
    url.searchParams.set('code_challenge', 'server-owned-proof')
    url.searchParams.set('code_challenge_method', 'S256')
  }
  return url
}

function validate(provider: ExternalBrowserProvider, url: URL, site = origin) {
  return validateExternalBrowserAuthorizationUrl({
    provider,
    value: url.toString(),
    clientId: 'synthetic-client',
    origin: site,
  })
}

describe('server-owned external browser authorization', () => {
  for (const provider of providers) {
    test(`${provider} requires explicit browser capability, provider readiness and real legal consent`, () => {
      const configured = {
        external_oauth_browser: true,
        [`${provider}_oauth`]: true,
        [`${provider}_client_id`]: 'synthetic-client',
      }
      assert.deepEqual(
        buildExternalBrowserStartRequest(provider, configured, false),
        { accepted_legal: false }
      )
      for (const change of [
        { external_oauth_browser: false },
        { external_oauth_browser: undefined },
        { [`${provider}_oauth`]: false },
        { [`${provider}_client_id`]: '' },
        { [`${provider}_client_id`]: ' ' },
        { user_agreement_enabled: true },
        { privacy_policy_enabled: true },
      ]) {
        assert.equal(
          buildExternalBrowserStartRequest(
            provider,
            { ...configured, ...change },
            false
          ),
          null
        )
      }
      assert.deepEqual(
        buildExternalBrowserStartRequest(
          provider,
          { ...configured, user_agreement_enabled: true },
          true
        ),
        { accepted_legal: true }
      )
      assert.equal(buildExternalBrowserStartRequest(provider, null, true), null)
    })

    test(`${provider} retains server parameters and every allowed same-origin callback`, () => {
      for (const callback of EXTERNAL_BROWSER_PRESETS[provider].callbacks) {
        const url = authorizationUrl(provider)
        url.searchParams.set('redirect_uri', `${origin}${callback}`)
        url.searchParams.set('provider_option', 'keep-the-server-value')
        assert.equal(validate(provider, url), url.toString())
      }
    })

    test(`${provider} rejects changed source, callback, client, scope, state and duplicate parameters`, () => {
      for (const mutate of [
        (url: URL) => {
          url.hostname += '.attacker.test'
        },
        (url: URL) => {
          url.protocol = 'http:'
        },
        (url: URL) => {
          url.pathname = '/other'
        },
        (url: URL) => {
          url.username = 'user'
        },
        (url: URL) => {
          url.hash = 'unsafe'
        },
        (url: URL) => {
          url.searchParams.set('client_id', 'other')
        },
        (url: URL) => {
          url.searchParams.set('response_type', 'token')
        },
        (url: URL) => {
          url.searchParams.set('scope', 'extra')
        },
        (url: URL) => {
          url.searchParams.delete('state')
        },
        (url: URL) => {
          url.searchParams.append('state', 'duplicate')
        },
        ...[
          `https://foreign.example.test/oauth/${provider}`,
          `${origin}/core/v1/auth/oauth/${provider}/callback`,
          `${origin}/oauth/${provider}?token=unsafe`,
          `${origin}/oauth/${provider}#unsafe`,
          `https://user:pass@site.example.test/oauth/${provider}`,
        ].map((callback) => (url: URL) => {
          url.searchParams.set('redirect_uri', callback)
        }),
      ]) {
        const url = authorizationUrl(provider)
        mutate(url)
        assert.throws(() => validate(provider, url))
      }
    })
  }

  test('GitHub and Discord require S256, while LinuxDO must use its confidential-code protocol', () => {
    for (const provider of providers) {
      const url = authorizationUrl(provider)
      if (provider === 'linuxdo') {
        url.searchParams.set('code_challenge', 'unsupported-proof')
      } else {
        url.searchParams.set('code_challenge_method', 'plain')
      }
      assert.throws(() => validate(provider, url))
      if (provider !== 'linuxdo') {
        url.searchParams.set('code_challenge_method', 'S256')
        url.searchParams.delete('code_challenge')
        assert.throws(() => validate(provider, url))
      }
    }
  })

  test('Discord cannot silently select a non-configurable direct API callback', () => {
    const url = authorizationUrl('discord')
    url.searchParams.set('redirect_uri', `${origin}/api/oauth/discord`)
    assert.throws(() => validate('discord', url))
  })

  test('only explicit local development allows loopback HTTP callback origin', () => {
    const url = authorizationUrl('github')
    const site = 'http://127.0.0.1:4174'
    url.searchParams.set('redirect_uri', `${site}/oauth/github`)
    assert.throws(() => validate('github', url, site))
    assert.equal(
      validateExternalBrowserAuthorizationUrl({
        provider: 'github',
        value: url.toString(),
        clientId: 'synthetic-client',
        origin: site,
        allowLocalDevelopment: true,
      }),
      url.toString()
    )
    assert.throws(() =>
      validateExternalBrowserAuthorizationUrl({
        provider: 'github',
        value: url.toString(),
        clientId: 'synthetic-client',
        origin: 'http://public.example.test',
        allowLocalDevelopment: true,
      })
    )
  })

  test('browser start parses an actual URL response, never native credentials or unsuccessful JSON', () => {
    assert.equal(
      parseBrowserAuthorizationResponse({
        success: true,
        data: { authorization_url: 'https://provider.example.test' },
      }),
      'https://provider.example.test'
    )
    for (const value of [
      null,
      { success: false, data: { authorization_url: 'https://provider.test' } },
      { success: true, data: { access_token: 'native-token' } },
      { success: true, data: { authorization_url: 123 } },
    ]) {
      assert.throws(() => parseBrowserAuthorizationResponse(value))
    }
  })
})

describe('external MFA callback search', () => {
  test('accepts the router numeric marker and preserves the Discord issuer', () => {
    const search = oauthCallbackSearchSchema.parse({
      external_mfa: 1,
      iss: 'https://discord.com',
      ignored: 'never-forward-this',
    })
    assert.equal(search.external_mfa, '1')
    assert.equal(search.iss, 'https://discord.com')
    assert.equal('ignored' in search, false)
    for (const provider of ['google', ...providers]) {
      assert.equal(isExternalMFAContinuation(provider, search), true)
    }
  })

  test('only four built-ins and an unambiguous marker bypass missing-code handling', () => {
    for (const provider of ['oidc', 'telegram', 'custom', 'GOOGLE']) {
      assert.equal(
        isExternalMFAContinuation(
          provider,
          oauthCallbackSearchSchema.parse({ external_mfa: '1' })
        ),
        false
      )
    }
    for (const search of [
      {},
      { external_mfa: 'true' },
      { external_mfa: 2 },
      { external_mfa: '1', code: 'code' },
      { external_mfa: '1', state: 'state' },
      { external_mfa: '1', error: 'denied' },
    ]) {
      assert.equal(
        isExternalMFAContinuation(
          'google',
          oauthCallbackSearchSchema.parse(search)
        ),
        false
      )
    }
  })
})
