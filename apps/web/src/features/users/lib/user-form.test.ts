/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  EMPTY_PERMISSION_CATALOG,
  type PermissionCatalog,
} from '@/lib/admin-permissions'

import { transformFormDataToPayload } from './user-form'

const catalog: PermissionCatalog = {
  resources: [
    {
      resource: 'channel',
      label_key: 'Channels',
      actions: [
        { action: 'read', label_key: 'Read', description_key: '' },
        {
          action: 'secret_view',
          label_key: 'View secrets',
          description_key: '',
        },
      ],
    },
  ],
  roles: [
    {
      key: 'admin',
      name: 'Administrator',
      built_in: true,
      superuser: false,
      grants: { channel: { read: true, secret_view: false } },
    },
  ],
}

test('creation and update omit permission writes when the catalog is unavailable', () => {
  for (const userId of [undefined, 41]) {
    for (const unavailable of [undefined, EMPTY_PERMISSION_CATALOG]) {
      const payload = transformFormDataToPayload(
        { username: 'synthetic-admin', role: 10, admin_permissions: {} },
        userId,
        unavailable
      )
      assert.equal(Object.hasOwn(payload, 'admin_permissions'), false)
    }
  }
})

test('a complete catalog preserves overrides and explicit false on creation and update', () => {
  for (const userId of [undefined, 41]) {
    const payload = transformFormDataToPayload(
      {
        username: 'synthetic-admin',
        role: 10,
        admin_permissions: { channel: { read: false } },
      },
      userId,
      catalog
    )
    assert.deepEqual(payload.admin_permissions, {
      channel: { read: false, secret_view: false },
    })
  }
})

test('ordinary user changes never include an administrator permission matrix', () => {
  const payload = transformFormDataToPayload(
    { username: 'synthetic-user', role: 1 },
    41,
    catalog
  )
  assert.equal(Object.hasOwn(payload, 'admin_permissions'), false)
})
