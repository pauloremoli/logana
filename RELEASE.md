# Release Process

## Overview

Releases are driven by Git tags and automated with [cargo-dist](https://axodotdev.github.io/cargo-dist) (v0.31.0).
Pushing a tag matching a semver pattern (e.g. `vX.Y.Z`) to `main` triggers the
[release workflow](.github/workflows/release.yml), which fully automates:

1. Cross-compiles binaries for all supported targets via cargo-dist
2. Creates a GitHub Release with binaries, SHA-256 checksums, and installers
3. Publishes the crate to crates.io
4. Updates the Homebrew formula in the [pauloremoli/homebrew-logana](https://github.com/pauloremoli/homebrew-logana) tap

> **Note:** The full CI suite (fmt → clippy → tests → coverage) runs separately in the
> [rust workflow](.github/workflows/rust.yml) on every push and pull request to `main`.
> The release workflow does not re-run CI — make sure CI is green before tagging.

---

## Prerequisites

### Repository secrets

Configure these once in **Settings → Secrets and variables → Actions**:

| Secret | How to obtain |
|---|---|
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New Token (scope: `publish-new` + `publish-update`) |
| `HOMEBREW_TAP_TOKEN` | GitHub → Settings → Developer settings → Personal access tokens → Tokens (classic) → New token → `repo` scope |

`GITHUB_TOKEN` is provided automatically by GitHub Actions — no manual setup needed.

### First-time crates.io setup

The crate name must be reserved before the first automated publish:

```sh
cargo publish --dry-run   # verify everything looks correct
cargo publish             # claim the name on crates.io
```

After this the release workflow handles all subsequent publishes.

---

## Cutting a release

### 1. Update the version

- Bump the version in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

- Update the CHANGELOG

Update the for next release section to the version being released.


- Commit the version bump:

```sh
git add Cargo.toml Cargo.lock
git commit -m "version vX.Y.Z"
git push
```

### 2. Tag and push

```sh
git tag vX.Y.Z
git push origin vX.Y.Z
```

This triggers the release workflow. Monitor progress in the **Actions** tab.

### 3. Verify the release

Once the workflow completes, check:

- [ ] GitHub Release exists at `https://github.com/pauloremoli/logana/releases/tag/vX.Y.Z`
- [ ] All six platform binaries are attached (Linux x86-64/ARM64/musl, macOS Intel/Apple Silicon, Windows x86-64)
- [ ] `install.sh`, `install.ps1`, and the Windows MSI are attached
- [ ] crates.io shows the new version at `https://crates.io/crates/logana`
- [ ] Homebrew formula in `pauloremoli/homebrew-logana` has the updated version and sha256 hashes

---

## Supported targets

| Target | Platform |
|---|---|
| `x86_64-unknown-linux-gnu` | Linux (x86-64, glibc) |
| `x86_64-unknown-linux-musl` | Linux (x86-64, musl/Alpine) |
| `aarch64-unknown-linux-gnu` | Linux (ARM64) |
| `x86_64-apple-darwin` | macOS (Intel) |
| `aarch64-apple-darwin` | macOS (Apple Silicon) |
| `x86_64-pc-windows-msvc` | Windows (x86-64) |

---

## Rolling back a release

If a release needs to be pulled:

1. Delete the GitHub Release and tag via the UI (or `gh release delete vX.Y.Z && git push --delete origin vX.Y.Z`).
2. Yank the crates.io version: `cargo yank --version X.Y.Z`
3. Revert the Homebrew formula in `pauloremoli/homebrew-logana` to the previous version.

GUI and full installers create icon-bearing desktop launchers: Windows Desktop
and Start Menu shortcuts (MSI and PowerShell), Linux application menu entries
and a shortcut when a Desktop directory exists, and `~/Applications/Logana.app`
with a Desktop link on macOS. The TUI-only installer remains terminal-only.
After generating global installers locally, run
`python3 scripts/prepare-desktop-installers.py` before distributing them.
The release workflow runs this automatically.

## Debian / Ubuntu packages

Release CI builds `logana` (TUI), `logana-gui` (GUI), and `logana-full`
(both frontends) as `.deb` packages for amd64 and arm64 on Ubuntu 24.04.
Pull requests also build and install-test the packages, available as workflow
artifacts. Tagged releases attach the packages and SHA-256 checksums to GitHub
Releases; package build failures block publication.

Download the package matching your architecture (`dpkg --print-architecture`)
and install it with APT so runtime dependencies are installed too:

```sh
sudo apt install ./logana_0.7.7-1_amd64.deb
```

The GUI editions install an application menu launcher and icon. The full
package conflicts with the separate editions because it owns the same files.
These are downloadable installers, not an APT repository or PPA. Compatibility
with Debian/Ubuntu releases depends on their available runtime library versions;
CI verifies installation on Ubuntu 24.04.

To build locally, install `cargo-deb` 3.8.0 and the system dependencies listed in
the release workflow, then run `cargo deb -p logana --locked --strip` (or select
`logana-gui` / `logana-full`). Output is written to `target/debian/`.
