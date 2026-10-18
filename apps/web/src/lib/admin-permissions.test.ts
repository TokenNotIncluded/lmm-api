/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  normalizeAdminPermissions,
  parsePermissionCatalogResponse,
  type PermissionCatalog,
} from './admin-permissions'

const catalog: PermissionCatalog = {
  resources: [
    {
      resource: 'channel',
      label_key: 'Channels',
      actions: [
        { action: 'read', label_key: 'Read', description_key: 'Read channels' },
        {
          action: 'secret_view',
          label_key: 'View secrets',
          description_key: 'View channel secrets',
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

test('accepts the server-owned catalog and preserves explicit denials', () => {
  const parsed = parsePermissionCatalogResponse({
    success: true,
    message: '',
    data: catalog,
  })
  assert.deepEqual(parsed, catalog)
  assert.deepEqual(
    normalizeAdminPermissions({ channel: { read: false } }, parsed),
    { channel: { read: false, secret_view: false } }
  )
})

test('accepts additional server-owned resources without a client allowlist', () => {
  const expanded = structuredClone(catalog)
  expanded.resources.push({
    resource: 'future_resource',
    label_key: 'Future resource',
    actions: [{ action: 'read', label_key: 'Read', description_key: '' }],
  })
  expanded.roles[0].grants.future_resource = { read: false }
  assert.deepEqual(
    parsePermissionCatalogResponse({ success: true, data: expanded }),
    expanded
  )
})

test('rejects failed, missing, malformed and incomplete catalogs', () => {
  const missingBaseline = structuredClone(catalog)
  delete missingBaseline.roles[0].grants.channel.secret_view
  const duplicateAction = structuredClone(catalog)
  duplicateAction.resources[0].actions.push(
    duplicateAction.resources[0].actions[0]
  )
  const duplicateResource = structuredClone(catalog)
  duplicateResource.resources.push(duplicateResource.resources[0])
  const duplicateRole = structuredClone(catalog)
  duplicateRole.roles.push(duplicateRole.roles[0])

  for (const response of [
    null,
    {},
    { success: false, message: 'Unauthorized', data: catalog },
    { success: 'true', data: catalog },
    { success: true },
    { success: true, data: { resources: [], roles: [] } },
    { success: true, data: { ...catalog, resources: {} } },
    { success: true, data: { ...catalog, roles: [] } },
    {
      success: true,
      data: {
        ...catalog,
        roles: [{ ...catalog.roles[0], grants: { channel: { read: 'true' } } }],
      },
    },
    ...[missingBaseline, duplicateAction, duplicateResource, duplicateRole].map(
      (data) => ({ success: true, data })
    ),
  ]) {
    assert.throws(
      () => parsePermissionCatalogResponse(response),
      /permission catalog is unavailable/
    )
  }
})
