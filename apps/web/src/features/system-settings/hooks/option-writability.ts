import type { SystemOptionsCapabilities, WritableSystemOptionKey } from '../types'
import { WRITABLE_SYSTEM_OPTION_KEYS } from '../types'

const announcementKeys = [
  'console_setting.announcements',
  'Announcements',
] as const satisfies readonly WritableSystemOptionKey[]

/** Intersect the server's live capability with the UI's narrow safe-key set. */
// @context-fold begin writable-option-key-resolution
export function writableOptionKey(
  capabilities: SystemOptionsCapabilities | undefined,
  requestedKey: string
): string | undefined {
  if (!capabilities) return requestedKey

  if (
    capabilities.read_only === true ||
    capabilities.writable_keys === undefined
  ) {
    return undefined
  }

  const writable = new Set(
    capabilities.writable_keys.filter((key): key is WritableSystemOptionKey =>
      WRITABLE_SYSTEM_OPTION_KEYS.includes(key as WritableSystemOptionKey)
    )
  )
  if (!writable.has(requestedKey as WritableSystemOptionKey)) {
    if (
      (announcementKeys as readonly string[]).includes(requestedKey)
    ) {
      return announcementKeys.find((key) => writable.has(key))
    }
    return undefined
  }
  return requestedKey
}
// @context-fold end writable-option-key-resolution

export function settingsUseRestrictedView(
  capabilities: SystemOptionsCapabilities | undefined
) {
  return capabilities !== undefined
}

export function isOptionWritable(
  capabilities: SystemOptionsCapabilities | undefined,
  key: string
) {
  return writableOptionKey(capabilities, key) !== undefined
}
