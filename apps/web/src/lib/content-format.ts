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
export function isHttpUrl(value: string): boolean {
  return isSafeHttpUrl(value)
}

export function isSafeHttpUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return (
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      url.hostname !== '' &&
      !url.username &&
      !url.password
    )
  } catch {
    return false
  }
}

type LiteralIpAddress =
  | { version: 4; octets: readonly number[] }
  | { version: 6; groups: readonly number[] }

function parseIpv4(value: string): readonly number[] | null {
  if (!/^\d+(?:\.\d+){3}$/.test(value)) return null
  const octets = value.split('.').map(Number)
  return octets.every((octet) => Number.isInteger(octet) && octet <= 255)
    ? octets
    : null
}

function parseIpv6(value: string): readonly number[] | null {
  if (value.includes('%')) return null

  let address = value
  if (address.includes('.')) {
    const separator = address.lastIndexOf(':')
    if (separator === -1) return null
    const octets = parseIpv4(address.slice(separator + 1))
    if (!octets) return null
    address = `${address.slice(0, separator)}:${(
      (octets[0] << 8) |
      octets[1]
    ).toString(16)}:${((octets[2] << 8) | octets[3]).toString(16)}`
  }

  const compression = address.indexOf('::')
  if (compression !== -1 && compression !== address.lastIndexOf('::')) {
    return null
  }

  const parseGroups = (part: string): number[] => {
    if (!part) return []
    const groups = part.split(':')
    if (groups.some((group) => !/^[0-9a-f]{1,4}$/i.test(group))) return []
    return groups.map((group) => Number.parseInt(group, 16))
  }

  if (compression === -1) {
    const groups = parseGroups(address)
    return groups.length === 8 ? groups : null
  }

  const left = parseGroups(address.slice(0, compression))
  const right = parseGroups(address.slice(compression + 2))
  if (left.length + right.length >= 8) return null
  return [...left, ...Array(8 - left.length - right.length).fill(0), ...right]
}

function parseLiteralIp(hostname: string): LiteralIpAddress | null {
  const value =
    hostname.startsWith('[') && hostname.endsWith(']')
      ? hostname.slice(1, -1)
      : hostname
  if (value.includes(':')) {
    const groups = parseIpv6(value)
    return groups ? { version: 6, groups } : null
  }
  const octets = parseIpv4(value)
  return octets ? { version: 4, octets } : null
}

function blockedIpv4(octets: readonly number[]): boolean {
  const [a, b, c] = octets
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 0 && c === 0) ||
    (a === 192 && b === 0 && c === 2) ||
    (a === 192 && b === 88 && c === 99) ||
    (a === 192 && b === 168) ||
    (a === 198 && b >= 18 && b <= 19) ||
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113) ||
    a >= 224
  )
}

function matchesIpv6Prefix(
  groups: readonly number[],
  prefix: readonly number[],
  bits: number
): boolean {
  const fullGroups = Math.floor(bits / 16)
  for (let index = 0; index < fullGroups; index += 1) {
    if (groups[index] !== prefix[index]) return false
  }
  const remainingBits = bits % 16
  if (remainingBits === 0) return true
  const mask = (0xffff << (16 - remainingBits)) & 0xffff
  return (groups[fullGroups] & mask) === (prefix[fullGroups] & mask)
}

function blockedIpv6(groups: readonly number[]): boolean {
  const mappedIpv4 =
    groups.slice(0, 5).every((group) => group === 0) && groups[5] === 0xffff
  return (
    mappedIpv4 ||
    (groups[0] & 0xe000) !== 0x2000 ||
    matchesIpv6Prefix(groups, [0x100, 0, 0, 0, 0, 0, 0, 0], 64) ||
    matchesIpv6Prefix(groups, [0x2001, 0, 0, 0, 0, 0, 0, 0], 23) ||
    matchesIpv6Prefix(groups, [0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32) ||
    matchesIpv6Prefix(groups, [0x2002, 0, 0, 0, 0, 0, 0, 0], 16) ||
    matchesIpv6Prefix(groups, [0x3e00, 0, 0, 0, 0, 0, 0, 0], 16) ||
    matchesIpv6Prefix(groups, [0x3ffe, 0, 0, 0, 0, 0, 0, 0], 16) ||
    matchesIpv6Prefix(groups, [0x3fff, 0, 0, 0, 0, 0, 0, 0], 20) ||
    matchesIpv6Prefix(groups, [0x5f00, 0, 0, 0, 0, 0, 0, 0], 16) ||
    matchesIpv6Prefix(groups, [0x64, 0xff9b, 0, 0, 0, 0, 0, 0], 96) ||
    matchesIpv6Prefix(groups, [0x64, 0xff9b, 1, 0, 0, 0, 0, 0], 48) ||
    matchesIpv6Prefix(groups, [0xfc00, 0, 0, 0, 0, 0, 0, 0], 7) ||
    matchesIpv6Prefix(groups, [0xfe80, 0, 0, 0, 0, 0, 0, 0], 10) ||
    matchesIpv6Prefix(groups, [0xff00, 0, 0, 0, 0, 0, 0, 0], 8)
  )
}

function isLocalHostname(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, '')
  return (
    host === 'localhost' ||
    host === 'local' ||
    host === 'internal' ||
    host.endsWith('.localhost') ||
    host.endsWith('.local') ||
    host.endsWith('.internal') ||
    host.includes('%')
  )
}

/**
 * Validate a configured public document URL without resolving DNS in the browser.
 * Literal special/private addresses and local-only hostnames are denied; public
 * hostnames remain allowed because their DNS answers are outside this process.
 */
export function isSafePublicHttpUrl(value: string): boolean {
  try {
    const url = new URL(value)
    if (
      url.protocol !== 'https:' ||
      !url.hostname ||
      url.username ||
      url.password ||
      isLocalHostname(url.hostname)
    ) {
      return false
    }
    const literal = parseLiteralIp(url.hostname)
    if (!literal) return true
    return literal.version === 4
      ? !blockedIpv4(literal.octets)
      : !blockedIpv6(literal.groups)
  } catch {
    return false
  }
}

export function isSafeResourceUrl(value: string): boolean {
  const trimmed = value.trim()
  if (trimmed.startsWith('/') && !trimmed.startsWith('//')) {
    return !trimmed.includes('\\') && !trimmed.includes('\0')
  }
  return isSafeHttpUrl(trimmed)
}

export function isLikelyHtml(value: string): boolean {
  return /<!doctype html|<html[\s>]|<head[\s>]|<body[\s>]|<style[\s>]|<script[\s>]|<\/?[a-z][\s\S]*>/i.test(
    value
  )
}
