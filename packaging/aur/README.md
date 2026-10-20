# LMM API AUR packages

Two independently installable binary packages:

| AUR package | Program | Service |
| --- | --- | --- |
| `lmm-api-core-bin` | `/usr/bin/lmm-core` | `lmm-core.service` |
| `lmm-api-extensions-bin` | `/usr/bin/lmm-extensions` | `lmm-extensions.service` |

Both use the [native package filesystem and configuration contract](../linux/README.md).
Extensions do not depend on the Core package, and installing either package does
not enable or restart a service. The packages do not replace the old
`lmm-api-go-bin` or `lmm-api-web-bin` packages automatically; an existing site must
finish its reviewed data migration and traffic switch before removing its old
service.

## Generate a release recipe

Use the actual component archive and its trusted SHA-256 from the release build.
The generator reuses the portable-archive and native ELF checks, renders the
canonical systemd/sysusers/RPC templates, and generates `.SRCINFO` with `makepkg`.
It does not build application code, install software, or publish anything.

```sh
python3 -B packaging/aur/prepare.py \
  --component core \
  --tag microservices-v0.1.0-rc.1 \
  --revision "$VERIFIED_SOURCE_SHA" \
  --archive "$CORE_RELEASE_ARCHIVE" \
  --sha256 "$TRUSTED_CORE_ARCHIVE_SHA256" \
  --output packaging/aur/lmm-api-core-bin
```

Repeat with `--component extensions`, its own archive/hash, and output directory
`packaging/aur/lmm-api-extensions-bin`. Only architectures represented by verified
input archives appear in the recipe. Add repeated `--archive` and `--sha256`
arguments in matching order when a later release has more than one architecture.
The first release is x86_64 only. The upstream version `0.1.0-rc.1` becomes
Arch's `pkgver=0.1.0rc1`, so the later stable `0.1.0` sorts after it.

An output directory must not already exist. Generate updates into a separate
scratch directory, review the changes, and copy the generated source files to
the maintained recipe. This avoids replacing another task's uncommitted work.
Never publish a recipe with a placeholder checksum or a download URL that is not
publicly available.

## Check and publish

After the GitHub release assets are public, run `makepkg --cleanbuild` in each
generated recipe directory as a normal user. This checks all local assets and
downloads the archive using the same URL AUR users will use. Review the resulting
package contents and dependencies with `namcap` and `bsdtar -tf`.

The AUR repositories contain only generated source files: `PKGBUILD`, `.SRCINFO`,
the systemd/sysusers/RPC files, `INSTALL.md`, and `BUILDINFO.json`. Do not upload
downloaded archives, binary packages, `src/`, `pkg/`, or credentials. Commit these
reviewed source files to the matching package's AUR Git repository and push its
`master` branch. Read the remote commit and public package metadata back after
the push; AUR RPC caches can take time to update.

Public upstream guidance:
[AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines),
[PKGBUILD](https://man.archlinux.org/man/PKGBUILD.5.en), and
[.SRCINFO](https://wiki.archlinux.org/title/.SRCINFO).
