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
import { lstat, readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const distDirectory = path.resolve(scriptDirectory, '..', 'dist')
const indexPath = path.join(distDirectory, 'index.html')
const maxInitialGzipBytes = 768 * 1024
const maxSingleAssetGzipBytes = 300 * 1024

const index = await readFile(indexPath, 'utf8')
const references = new Set(
  [
    ...index.matchAll(
      /<(?:script|link)\b[^>]*(?:src|href)="(\/static\/[^"?#]+)"/g
    ),
  ].map((match) => match[1])
)

if (references.size === 0) {
  throw new Error('production index does not reference any static assets')
}

let totalGzipBytes = 0
const measurements = []
for (const reference of references) {
  const assetPath = path.resolve(distDirectory, `.${reference}`)
  if (!assetPath.startsWith(`${distDirectory}${path.sep}`)) {
    throw new Error(`initial asset escapes dist: ${reference}`)
  }
  const info = await lstat(assetPath)
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error(`initial asset is not a regular file: ${reference}`)
  }
  const bytes = await readFile(assetPath)
  const gzipBytes = gzipSync(bytes, { level: 9 }).byteLength
  if (gzipBytes > maxSingleAssetGzipBytes) {
    throw new Error(
      `initial asset exceeds ${maxSingleAssetGzipBytes} gzip bytes: ${reference} (${gzipBytes})`
    )
  }
  totalGzipBytes += gzipBytes
  measurements.push({ reference, rawBytes: bytes.byteLength, gzipBytes })
}

if (totalGzipBytes > maxInitialGzipBytes) {
  throw new Error(
    `initial bundle exceeds ${maxInitialGzipBytes} gzip bytes: ${totalGzipBytes}`
  )
}

measurements.sort((left, right) => right.gzipBytes - left.gzipBytes)
for (const measurement of measurements) {
  console.log(
    `${measurement.gzipBytes}\t${measurement.rawBytes}\t${measurement.reference}`
  )
}
console.log(
  `initial_bundle_gzip_bytes=${totalGzipBytes} limit=${maxInitialGzipBytes}`
)
