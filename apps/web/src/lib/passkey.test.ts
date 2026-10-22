/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { isPasskeySupported } from './passkey.ts'

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  'navigator'
)

function installBrowser(
  secure = true,
  credentials: Record<string, unknown> = {
    create: async () => null,
    get: async () => null,
  }
) {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      isSecureContext: secure,
      PublicKeyCredential: class {
        static async isConditionalMediationAvailable() {
          return false
        }
        static async isUserVerifyingPlatformAuthenticatorAvailable() {
          return false
        }
      },
    },
  })
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { credentials },
  })
}

afterEach(() => {
  for (const [key, descriptor] of [
    ['window', originalWindow],
    ['navigator', originalNavigator],
  ] as const) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('a browser without platform biometrics still supports roaming keys', async () => {
  installBrowser()
  assert.equal(await isPasskeySupported(), true)
})

test('an insecure page cannot offer WebAuthn', async () => {
  installBrowser(false)
  assert.equal(await isPasskeySupported(), false)
})

test('missing WebAuthn methods fail closed', async () => {
  for (const credentials of [{}, { get: async () => null }]) {
    installBrowser(true, credentials)
    assert.equal(await isPasskeySupported(), false)
  }
})

test('server rendering does not advertise browser support', async () => {
  Reflect.deleteProperty(globalThis, 'window')
  assert.equal(await isPasskeySupported(), false)
})
