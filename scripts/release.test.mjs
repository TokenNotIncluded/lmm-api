import assert from 'node:assert/strict'
import test from 'node:test'

import {
  extractReleaseNotes,
  finalizeChangelog,
  parseSemver,
  planVersion,
  updateCargoLock,
  updateCargoToml,
  updateSrcinfoVersion,
} from './release.mjs'

test('stable semantic versions reject prereleases and leading zeroes', () => {
  assert.deepEqual(parseSemver('1.2.3'), { major: 1, minor: 2, patch: 3, value: '1.2.3' })
  assert.throws(() => parseSemver('1.2.3-rc.1'), /invalid stable semantic version/)
  assert.throws(() => parseSemver('01.2.3'), /invalid stable semantic version/)
})

test('patch bump follows the current release line and avoids an existing tag', () => {
  assert.deepEqual(planVersion('0.1.0', 'patch', undefined, ['v0.1.0', 'v0.1.1', 'v0.13.2']), {
    baseline: '0.1.1',
    target: '0.1.2',
  })
  assert.equal(planVersion('0.1.1', 'minor', undefined, ['v0.2.0', 'v0.13.2']).target, '0.3.0')
  assert.equal(planVersion('0.1.1', 'major', undefined, ['v1.0.0-rc.24']).target, '1.0.0')
  assert.throws(
    () => planVersion('0.1.0', 'patch', '0.1.1', ['v0.1.1']),
    /must be newer than 0.1.1/
  )
})

test('changelog finalization preserves notes and maintains comparison links', () => {
  const input = `# Changelog

Intro.

## Unreleased

- Added release automation.

### Verification

- Release metadata tests pass.

[Unreleased]: https://example.invalid/old
[0.1.1]: https://example.invalid/older
`
  const output = finalizeChangelog(input, '0.1.2', '2026-08-13', '0.1.1')
  assert.match(output, /^## Unreleased$/m)
  assert.match(output, /^## \[0\.1\.2\] - 2026-08-13$/m)
  assert.match(output, /\[Unreleased\]: .*compare\/v0\.1\.2\.\.\.HEAD/)
  assert.match(output, /\[0\.1\.2\]: .*compare\/v0\.1\.1\.\.\.v0\.1\.2/)
  assert.equal(
    extractReleaseNotes(output, '0.1.2'),
    '- Added release automation.\n\n### Verification\n\n- Release metadata tests pass.'
  )
})

test('an empty Unreleased section cannot be published', () => {
  assert.throws(
    () =>
      finalizeChangelog(
        '# Changelog\n\n## Unreleased\n\n<!-- nothing yet -->\n',
        '0.1.2',
        '2026-08-13',
        '0.1.1'
      ),
    /Unreleased section is empty/
  )
})

test('Rust workspace and lock versions update without touching dependencies', () => {
  const toml = `[workspace]
members = []

[workspace.package]
edition = "2024"
version = "0.1.0"

[workspace.dependencies]
serde = "1.0.0"
`
  assert.match(updateCargoToml(toml, '0.1.2'), /\[workspace\.package\][^]*version = "0\.1\.2"/)
  assert.match(updateCargoToml(toml, '0.1.2'), /serde = "1\.0\.0"/)

  const names = [
    'lmm-api-rs',
    'lmm-application',
    'lmm-contracts',
    'lmm-db-migrate',
    'lmm-domain',
    'lmm-observability',
  ]
  const lock = names.map((name) => `[[package]]\nname = "${name}"\nversion = "0.1.0"\n`).join('\n')
  const updated = updateCargoLock(lock, '0.1.2')
  assert.equal((updated.match(/version = "0\.1\.2"/g) ?? []).length, names.length)
})

test('.SRCINFO version replacement updates artifact URLs consistently', () => {
  const srcinfo = `pkgbase = lmm-api-go-bin
\tpkgver = 0.1.0
\tprovides = lmm-api-go=0.1.0
\tsource = https://example.invalid/releases/download/v0.1.0/lmm-api-go-0.1.0.tar.gz
`
  const updated = updateSrcinfoVersion(srcinfo, '0.1.2')
  assert.doesNotMatch(updated, /0\.1\.0/)
  assert.equal((updated.match(/0\.1\.2/g) ?? []).length, 4)
})
