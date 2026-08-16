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
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

describe('mobile sign-in presentation', () => {
  test('features configured Google OAuth and keeps the narrow layout compact', () => {
    const authLayout = readFileSync(
      new URL('../auth-layout.tsx', import.meta.url),
      'utf8'
    )
    const providers = readFileSync(
      new URL('../components/oauth-providers.tsx', import.meta.url),
      'utf8'
    )
    const form = readFileSync(
      new URL('./components/user-auth-form.tsx', import.meta.url),
      'utf8'
    )
    const signIn = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8')

    assert.match(authLayout, /h-dvh/)
    assert.match(authLayout, /lg:absolute/)
    assert.match(authLayout, /max-w-md/)
    assert.match(providers, /IconGoogle/)
    assert.match(providers, /Continue with Google/)
    assert.match(providers, /t\('Or'\)/)
    assert.match(
      providers,
      /const showProviderDivider =\s+!featuredProvider\s+\|\|\s+otherProviders\.length > 0/
    )
    assert.match(providers, /showProviderDivider \? \(/)
    assert.match(providers, /grid-cols-2/)
    assert.match(form, /featureGoogle/)
    assert.match(form, /hasAlternativeLogin && passwordLoginEnabled/)
    assert.match(form, /\{t\('Or'\)\}/)
    assert.match(form, /auth-field h-11 rounded-xl/)
    assert.match(signIn, /hasRegistrationMethod\(status\)/)
  })
})
