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

For commercial licensing, please contact support@quantumnous.com
*/
import { z } from 'zod'

import type { AuthUser } from '@/stores/auth-store'

import { ROLE } from './roles'

export type AdminPermissionMatrix = Record<string, Record<string, boolean>>
export type AdminCapabilities = AdminPermissionMatrix

export const ADMIN_PERMISSION_RESOURCES = {
  CHANNEL: 'channel',
} as const

export const ADMIN_PERMISSION_ACTIONS = {
  READ: 'read',
  OPERATE: 'operate',
  WRITE: 'write',
  SENSITIVE_WRITE: 'sensitive_write',
  SECRET_VIEW: 'secret_view',
} as const

// The role whose baseline grants are used as defaults in the permission editor.
export const ADMIN_ROLE_KEY = 'admin'

// The permission catalog (resources, actions, labels and role baselines) is owned
// by the backend authz package and fetched from GET /api/authz/catalog. It is
// intentionally NOT duplicated here so the schema stays defined in one place.
// These types mirror the backend JSON shape.
export interface PermissionActionDef {
  action: string
  label_key: string
  description_key: string
}

export interface PermissionResourceDef {
  resource: string
  label_key: string
  actions: PermissionActionDef[]
}

export interface PermissionRoleDef {
  key: string
  name: string
  built_in: boolean
  superuser: boolean
  grants: AdminPermissionMatrix
}

export interface PermissionCatalog {
  resources: PermissionResourceDef[]
  roles: PermissionRoleDef[]
}

const permissionCatalogSchema = z
  .object({
    resources: z
      .array(
        z.object({
          resource: z.string().min(1),
          label_key: z.string().min(1),
          actions: z
            .array(
              z.object({
                action: z.string().min(1),
                label_key: z.string().min(1),
                description_key: z.string(),
              })
            )
            .min(1),
        })
      )
      .min(1),
    roles: z
      .array(
        z.object({
          key: z.string().min(1),
          name: z.string().min(1),
          built_in: z.boolean(),
          superuser: z.boolean(),
          grants: z.record(z.string(), z.record(z.string(), z.boolean())),
        })
      )
      .min(1),
  })
  .superRefine((catalog, context) => {
    const resources = catalog.resources.map((item) => item.resource)
    const roles = catalog.roles.map((item) => item.key)
    const baseline = catalog.roles.find((role) => role.key === ADMIN_ROLE_KEY)
    if (
      new Set(resources).size !== resources.length ||
      new Set(roles).size !== roles.length ||
      !baseline ||
      catalog.resources.some((resource) => {
        const actions = resource.actions.map((item) => item.action)
        return (
          new Set(actions).size !== actions.length ||
          actions.some(
            (action) =>
              typeof baseline.grants[resource.resource]?.[action] !== 'boolean'
          )
        )
      })
    ) {
      context.addIssue({
        code: 'custom',
        message: 'Incomplete permission catalog',
      })
    }
  })

const permissionCatalogResponseSchema = z.object({
  success: z.literal(true),
  data: permissionCatalogSchema,
})

export function parsePermissionCatalogResponse(
  value: unknown
): PermissionCatalog {
  const result = permissionCatalogResponseSchema.safeParse(value)
  if (!result.success) {
    throw new Error('Administrator permission catalog is unavailable')
  }
  return result.data.data
}

export const EMPTY_PERMISSION_CATALOG: PermissionCatalog = {
  resources: [],
  roles: [],
}

export function hasPermission(
  user: AuthUser | null | undefined,
  resource: string,
  action: string
): boolean {
  if (!user) return false
  if (user.role === ROLE.SUPER_ADMIN) return true
  return user.permissions?.admin_permissions?.[resource]?.[action] === true
}

// roleGrants returns the baseline grant matrix for the given role key.
export function roleGrants(
  catalog: PermissionCatalog,
  roleKey: string
): AdminPermissionMatrix {
  return catalog.roles.find((role) => role.key === roleKey)?.grants ?? {}
}

// normalizeAdminPermissions produces a full matrix for the catalog, filling any
// value missing from `value` with the admin role's baseline grant.
export function normalizeAdminPermissions(
  value: AdminPermissionMatrix | null | undefined,
  catalog: PermissionCatalog
): AdminPermissionMatrix {
  const baseline = roleGrants(catalog, ADMIN_ROLE_KEY)
  const normalized: AdminPermissionMatrix = {}
  for (const resource of catalog.resources) {
    const actions: Record<string, boolean> = {}
    for (const action of resource.actions) {
      actions[action.action] =
        value?.[resource.resource]?.[action.action] ??
        baseline[resource.resource]?.[action.action] ??
        false
    }
    normalized[resource.resource] = actions
  }
  return normalized
}
