# LMM API fork

LMM API is a maintained fork of [QuantumNous/new-api](https://github.com/QuantumNous/new-api).

- Upstream snapshot: commit `823e26304a396854ace30b52b98ec497c2dd9c36`, dated 2026-08-08
- License: GNU Affero General Public License v3.0 (`AGPL-3.0`)
- Local user-facing brand: `LMM API`
- Go module identity: `github.com/LIghtJUNction/api.lmm.best`
- Fork-specific modifications: `Copyright (C) 2026 LIghtJUNction`

The upstream copyright notices, attribution, `NOTICE`, and
`THIRD-PARTY-LICENSES.md` are preserved. Modified user interfaces must retain
the original-project link and attribution required by `NOTICE`.

New or fork-modified frontend source files retain the upstream header and may
add the separate LIghtJUNction modification notice described in `NOTICE`.

The current synchronization intentionally covers the Go backend subtree;
the shared frontend and repository-specific CI remain maintained at the root.

Upstream updates should be imported as reviewed, pinned snapshots. Compare the
new snapshot with the recorded commit, preserve local branding as a small
focused patch, retain compatibility identifiers, and run the relevant
validation before accepting the update.
