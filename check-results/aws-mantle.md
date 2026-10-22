# AWS Mantle: source and dependency checks

Recorded on 2026-10-22 UTC against the shared working tree based on
`451f50b07880051f4c0de43ba99b7c1e2e1b8089`. The file hashes below identify the
checked source; the base commit alone does not contain these changes. This is
development evidence, not a Rust execution result, financial acceptance or
live AWS certification. No deployment, credential provisioning or paid provider
request was performed.

## Executed checks

| Check | Result | Scope |
| --- | --- | --- |
| Official Rust 1.99.0 rustfmt, edition 2024, `skip_children=true`, `--check` | Exit 0 | Nine Mantle signer, relay and configuration/route integration files listed below |
| `git diff --check` | Exit 0 | Current working-tree diff |
| `python3 -B scripts/check-docs-brand.py --docs-only` | Exit 0, PASS | Maintained document paths, skill metadata and README badges |
| Python JSON parsing and relative-link existence checks | Exit 0 | All eight JSON examples and relative file links in `services/core/src/routing/README.md` |
| Actual Cargo dependency resolution | Exit 0 | Explicit pinned AWS dependencies, Rust 1.91 compatibility fallback |
| Cargo `metadata --locked --offline --no-deps` | Exit 0 | Both the shared manifest and the isolated Mantle-only manifest |

The formatter command was run from the repository root:

```sh
LD_LIBRARY_PATH=/tmp/lmm-rust-format/rustc-1.99.0-x86_64-unknown-linux-gnu/rustc/lib \
  /tmp/lmm-rust-format/rustfmt-1.99.0-x86_64-unknown-linux-gnu/rustfmt-preview/bin/rustfmt \
  --edition 2024 --config skip_children=true --check \
  services/core/src/relay/mantle.rs \
  services/core/src/relay/mantle_tests.rs \
  services/core/src/relay/types.rs \
  services/core/src/relay/mod.rs \
  services/core/src/model_http.rs \
  services/core/src/model_http/provider_policy.rs \
  services/core/src/model_http/routing.rs \
  services/core/src/model_http/pricing.rs \
  services/core/src/config.rs
```

The existing read-only Cargo/Rust compiler binaries reported version 1.99.1.
They were reused for dependency resolution and metadata only; no compiler,
package manager or full toolchain was installed for this change. Cargo used the
task-local cache `/tmp/lmm-mantle-cargo`. The manifest declares Rust 1.91 and pins
`aws-sigv4 = 1.4.1` with `sign-http,http1`, and `aws-credential-types = 1.2.13`
without default features. Version and API choices were checked against the
official registry metadata and the checksum-verified source archives.

The shared manifest also includes the separate plugin task's rquickjs dependency.
For an independently reviewable Mantle commit, a temporary copy removed only
that dependency and its comment. **The shared working-tree manifest was not
changed.** The actual isolated lock update was:

```sh
CARGO_HOME=/tmp/lmm-mantle-cargo \
RUSTC=/tmp/hireferry-rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/rustc \
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback \
  /tmp/hireferry-rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/cargo \
  update --offline -p lmm-core --manifest-path /tmp/lmm-mantle-only-lock/Cargo.toml
```

Cargo reported `Locking 0 packages to highest Rust 1.91 compatible versions` and
removed `relative-path 2.0.3`, `rquickjs 0.14.0`, `rquickjs-core 0.14.0` and
`rquickjs-sys 0.14.0`. Relative to the base lockfile, the isolated result adds
17 AWS-related dependency packages and removes/replaces **no existing package
version**. The existing locked `tracing 0.1.44` becomes a direct, std-only
dependency to suppress the signer's sensitive trace events locally; this adds
no package version.

The isolated metadata check was:

```sh
CARGO_HOME=/tmp/lmm-mantle-cargo \
RUSTC=/tmp/hireferry-rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/rustc \
  /tmp/hireferry-rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/cargo \
  metadata --locked --offline --format-version 1 --no-deps \
  --manifest-path /tmp/lmm-mantle-only-lock/Cargo.toml \
  > /tmp/lmm-mantle-only-lock/metadata.json
```

## Cross-owner source review

The relay owner reviewed the root agent's configuration, provider policy,
route construction and native price projection. Review found and corrected
four concrete issues before this record: disabled upstreams incorrectly required
unloaded secrets; distinct credential references could reinterpret the same
AWS document as an API key; bare-origin endpoint normalization prevented an
exact Mantle match; and configuration tests initially supplied JSON to the
TOML-only production reader. The tests now use a closed, test-only TOML writer;
production parsing was not widened.

All references, including disabled targets, participate in authentication-type
checks. Missing secrets on disabled upstreams are permitted. Loaded ordinary
credentials cannot be strict AWS credential documents or reuse an AWS access
key, secret key or session token from another reference. Private JSON files
retain the existing absolute-path, symlink, permission and device/inode origin
checks. Public pricing validates the dedicated Mantle binding; it does not treat
JSON as an API-key header. Existing OAuth catalog protocol restrictions were
retained.

The signer was separately checked against the pinned AWS crate source. In
particular, its non-exhaustive `SigningSettings` is initialized through
`Default`; AWS canonical-request TRACE output is suppressed inside the
synchronous signing call using a local discard dispatcher. The caller's
dispatcher is restored before returning. Final request bytes, region, host,
operation, content hash and any session token participate in signing. Production
cryptography is implemented by the official AWS crate, not a copied Go signer
or a local HMAC implementation.

## Source regression coverage awaiting execution

Six Mantle unit-test sources cover canonical endpoint bases and rejection,
an independently calculated fixed public-credential signature vector,
body/time/stream sensitivity, signed temporary session tokens, strict secret
parsing and redacted debug output, forbidden adapter combinations,
constant-time parsed-field reuse checks, transient credential fingerprints,
and suppression/restoration under an enabled external TRACE subscriber.

Provider-policy tests bind region, exact model/protocol/endpoint and exclude
mixed supplier contracts. A Unix configuration test uses actual private files,
a 5,000-byte synthetic session token and a lazy database pool with no connection
attempt; its cases cover separate and reused references/values, disabled
upstreams, wrong regions, symlinks and unsafe file modes. These remain **test
sources, not tests executed in this workspace**.

No Rust compilation, Cargo test, Clippy, PostgreSQL settlement test, live TLS
supplier call, AWS permission check or STS refresh test was performed for this
record. Formatting and Cargo metadata do not establish type correctness or
signing compatibility. Parent CI results must be read against their exact
published commit and must not be attributed to these hashes without matching
the actual inputs. The separate native DeepSeek/Ali ledger tests and their
runtime results remain a distinct acceptance scope.

## Checked input hashes

SHA-256 hashes identify the exact working-tree files read/formatted at this
point. The two temporary dependency files are the Mantle-only replacement
inputs for a snapshot; they differ from the shared plugin-inclusive manifest.
The temporary copies are dependency inputs, not a compiled artifact.

| Input | SHA-256 |
| --- | --- |
| `services/core/src/relay/mantle.rs` | `dcb45aebb6c2aa4eaa8896281efc907b61f5d20a3d87a919b56448e40ed07d7c` |
| `services/core/src/relay/mantle_tests.rs` | `350351592fff2726f0747e5dae72053f5d7ee7261e889045eee937bc7d7c4533` |
| `services/core/src/relay/types.rs` | `b105b37a8c3ee70ad4d2b7ce1bfee72bfef7766f23bff3bfde951ac12597f787` |
| `services/core/src/relay/mod.rs` | `12f9511da6ed064932cbdbabcb8dd48627647f074516648b95b41c2cb8874ade` |
| `services/core/src/model_http.rs` | `ed99e245688a964a8be6c6f6b19b954f5fbdbaca6261914c9d8098fee0cc8f17` |
| `services/core/src/model_http/provider_policy.rs` | `cae9141dcf43b173727dbd4bbc54929ad1ff3c6aae0a0f76baabedbef977b51f` |
| `services/core/src/model_http/routing.rs` | `05acae62b4ac1afb1544231363d18fadfd6c6fb1714c9a340d18af57d9f52a04` |
| `services/core/src/model_http/pricing.rs` | `bd409a853edbbf1f011fdc872a7f1f5d54c85a06f1f30c9e2402ca2418e5c5c5` |
| `services/core/src/config.rs` | `b89729655fb2c31b86d9cb9e22cfe098632a13a37c0b22d2ace75bd5a2afc1bc` |
| `services/core/src/routing/README.md` | `676cdd73089ba8ae213beaaee10abb5b48951e6cda140ce82119c61b3da4cf16` |
| `/tmp/lmm-mantle-only-lock/Cargo.toml` | `f0fc90ed40a7dcb8f6780618fb69a7287751efcb14f77f75d34a70b6490cfb76` |
| `/tmp/lmm-mantle-only-lock/Cargo.lock` | `7aaa7ba01d06f43a45920f87f7348e33158b56634d6b58bb40fbd89e727a2b79` |

Canonical behavior and source references are in
[`routing/README.md`](../services/core/src/routing/README.md#aws-mantle-messages).
The AWS crates retain their Apache-2.0 license; the repository's existing AGPL
attribution and upstream provenance remain applicable.
