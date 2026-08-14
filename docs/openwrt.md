# OpenWrt distribution and footprint

Hop v0.2 treats OpenWrt as a first-class release target while keeping router
packages small. The separate
[`luci-app-hop`](https://github.com/oslo254804746/luci-app-hop) repository is an
architecture-independent control package. It contains LuCI, procd/UCI, the
strict default-off startup config, and a verified core downloader. It does not
fetch source or compile Rust inside an OpenWrt SDK.

The Hop core is built once per supported CPU family in this repository and
published as GitHub Release assets:

```text
hop-server-linux-x86_64-musl.tar.gz
hop-server-linux-aarch64-musl.tar.gz
SHA256SUMS
```

Each archive contains a single static executable named `hop-server`. On the
first enabled start, the router selects the asset for `x86_64`/`amd64` or
`aarch64`/`arm64`, verifies its SHA-256 entry, runs the binary's version
self-check, and atomically installs it under `/etc/hop/core`. A failed download
or verification does not replace the current core.

## Package and release validation

The two repositories have distinct cloud checks:

| Boundary | Validation |
|---|---|
| Hop core | `cross` builds static `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` release archives; tag publication aggregates one `SHA256SUMS` |
| LuCI package | OpenWrt 24.10.4 SDK emits an IPK and OpenWrt 25.12.5 SDK emits an APK from the same `PKGARCH:=all` source |

The LuCI workflow uses one x86/64 SDK per packaging backend because the
control package is identical on every router architecture. It verifies the
official SDK checksum and records `luci-base`, `curl`, and `ca-bundle` as
runtime-only package metadata. It does not fetch or rebuild the LuCI feeds,
never checks out `hop-rs`, and never runs Cargo.

On 2026-08-14, the official OpenWrt 24.10.4 SDK produced the architecture-
independent IPK and the official OpenWrt 25.12.5 SDK produced the APK from the
same source. Both packages passed artifact recording and upload in the LuCI
repository workflow.

The core workflow runs the cross-architecture asset jobs for `dev`/`dev-*`
branch pushes, release tags, and manual dispatches. It checks the ELF machine,
rejects a dynamic program interpreter, executes `hop-server --version` through
`cross`, and verifies the archive member name. Every path assembles the exact
two archives and one
`SHA256SUMS` file as a complete release-candidate artifact; only a version tag
publishes that candidate to GitHub Releases.

UCI owns only enablement, startup config path, download release/version, and
service logging. Assets, credentials, Access Keys, and key-to-asset allowlists
remain exclusively in the Hop SQLite Catalog. The OpenWrt package has no
Node.js, Docker, built-in Admin Web, or Rust toolchain dependency.

## 2026-08-14 host baseline

Measured with `cargo build --release -p hop-server --locked` on x86_64 GNU/Linux
and `scripts/measure_openwrt_footprint.sh`:

| Metric | Result |
|---|---:|
| Release ELF before strip | 9,027,304 bytes |
| Host `strip` result | 6,854,888 bytes |
| Idle RSS | 9,392 KiB |
| Main SQLite file immediately after initialization | 4,096 bytes |
| SQLite WAL immediately after initialization | 148,352 bytes |
| Main database growth during a two-second idle window | 0 bytes |
| WAL growth during a two-second idle window | 0 bytes |

Final compressed core archive size is recorded by the release workflow rather
than inferred from the GNU host binary. RSS still needs confirmation on real
x86_64 and aarch64 OpenWrt devices.

The initial schema is committed through SQLite WAL. Hop does not perform
periodic Catalog writes while idle; Catalog changes, session state, health
results, Known Hosts, and explicitly configured retention work are the expected
write sources.

## Reproduce the host measurement

```bash
cargo build --release -p hop-server --locked
scripts/measure_openwrt_footprint.sh
```

The measurement starts an isolated instance with SSH bound to an ephemeral
loopback port and `api.enabled = false`, samples `/proc`, observes the SQLite
files, and removes its temporary directory.
