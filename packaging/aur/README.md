# AUR packages

Each backend ships its own provider binary. The Go package also provides the
canonical `lmm-api` command as a symlink to `lmm-api-go`, so systemd can use a
stable service contract while the provider remains replaceable.

| Backend | Stable source | Prebuilt release | Build from Git | Installed command |
| --- | --- | --- | --- | --- |
| Go | `lmm-api-go` | `lmm-api-go-bin` | `lmm-api-go-git` | `/usr/bin/lmm-api` → `lmm-api-go` |
| Rust | — | `lmm-api-rs-bin` | `lmm-api-rs-git` | `/usr/bin/lmm-api-rs` |

The independent prebuilt frontend is `lmm-api-web-bin`. It publishes the
verified static payload under `/srv/lmm-api-frontend`, atomically changes the
`current` link, reloads nginx, verifies the public page, and rolls back the link
if verification fails. It never restarts the backend service.

The Go packages currently retain a bundled frontend for safe transition and
legacy rollback compatibility. New production frontend releases use
`lmm-api-web-bin`; after deployed-package activation contracts no longer rely
on the bundled payload, it can be removed from Go packages in a separate
release. The Go service runs `/usr/bin/lmm-api serve` through the provider
symlink and keeps its private `/etc/lmm-api-go/lmm-api-go.env` configuration.

Install the canonical source package with `paru -S lmm-api-go`. Use
`lmm-api-go-bin` only when a matching signed GitHub release exists, or
`lmm-api-go-git` when explicitly following the moving `main` branch.

The Rust packages remain separate until the Rust backend satisfies the same
native CLI and production route contract. A Rust cutover must atomically
replace the provider symlink and pass the route gate first.

Run `bash packaging/aur/test-matrix.sh` after changing a `PKGBUILD`, and
regenerate each tracked `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`.
