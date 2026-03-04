# Release Process

## Overview

Releases are driven by Git tags. Pushing a tag matching `vMAJOR.MINOR.PATCH`
to `main` triggers the [release workflow](.github/workflows/release.yml), which:

1. Runs the full CI suite (fmt → clippy → tests)
2. Cross-compiles binaries for all supported targets
3. Creates a GitHub Release with binaries, SHA-256 checksums, and install scripts
4. Publishes the crate to crates.io
5. Updates the Homebrew formula in the [pauloremoli/homebrew-logana](https://github.com/pauloremoli/homebrew-logana) tap

---

## Prerequisites

### Repository secrets

Configure these once in **Settings → Secrets and variables → Actions**:

| Secret | How to obtain |
|---|---|
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New Token (scope: `publish-new` + `publish-update`) |
| `HOMEBREW_TAP_TOKEN` | GitHub → Profile picture → Settings → Developer settings → Personal access tokens → Tokens (classic) → Generate new token → select `repo` scope → copy the token |

### First-time crates.io setup

The crate must be reserved before the first automated publish:

```sh
cargo publish --dry-run   # verify everything looks correct
cargo publish             # claim the name on crates.io
```

After this the release workflow handles all subsequent publishes.

---

## Cutting a release

### 1. Update the version

Bump the version in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

Also update `pkg/aur/PKGBUILD` and `pkg/aur/.SRCINFO` to match:

```sh
# PKGBUILD
pkgver=X.Y.Z

# .SRCINFO
pkgver = X.Y.Z
source = logana-X.Y.Z.tar.gz::https://github.com/pauloremoli/logana/archive/vX.Y.Z.tar.gz
```

Commit the version bump:

```sh
git add Cargo.toml Cargo.lock pkg/aur/PKGBUILD pkg/aur/.SRCINFO
git commit -m "chore: bump version to vX.Y.Z"
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
- [ ] All five platform binaries are attached (Linux x86/ARM, macOS x86/ARM, Windows x86)
- [ ] `install.sh` and `install.ps1` are attached
- [ ] crates.io shows the new version at `https://crates.io/crates/logana`
- [ ] Homebrew formula in `pauloremoli/homebrew-logana` has the updated version and sha256 hashes

---

## AUR update

The AUR package is maintained manually. After a release:

1. Update `pkg/aur/PKGBUILD` — set `pkgver` and compute the new `sha256sums`:
   ```sh
   curl -L https://github.com/pauloremoli/logana/archive/vX.Y.Z.tar.gz | sha256sum
   ```
2. Update `pkg/aur/.SRCINFO` to match.
3. Push to the AUR repository:
   ```sh
   git clone ssh://aur@aur.archlinux.org/logana.git aur-logana
   cp pkg/aur/PKGBUILD pkg/aur/.SRCINFO aur-logana/
   cd aur-logana
   git add PKGBUILD .SRCINFO
   git commit -m "Update to vX.Y.Z"
   git push
   ```

---

## Nix update

After a release, update `pkg/nix/default.nix` with the new version and hashes:

```sh
# Source hash
nix-prefetch-url --unpack https://github.com/pauloremoli/logana/archive/vX.Y.Z.tar.gz

# Cargo dependencies hash (inside a nix-shell with cargo available)
cd /tmp && git clone --branch vX.Y.Z https://github.com/pauloremoli/logana
cd logana && cargo vendor
nix hash path vendor/
```

Update the `version`, `hash`, and `cargoHash` fields in `pkg/nix/default.nix`, then
submit a PR to [nixpkgs](https://github.com/NixOS/nixpkgs) or push to your overlay.

---

## Supported targets

| Target | Platform |
|---|---|
| `x86_64-unknown-linux-gnu` | Linux (x86-64) |
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
4. If the AUR was updated, push a revert commit there too.
