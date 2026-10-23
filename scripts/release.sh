#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Usage: $0 <version>"
    echo "  version: MAJOR.MINOR.PATCH (e.g. 0.4.0)"
    exit 1
}

if [[ $# -ne 1 ]]; then
    usage
fi

VERSION="$1"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: version must match MAJOR.MINOR.PATCH (got: '$VERSION')"
    exit 1
fi

if [[ ! -f "Cargo.toml" ]]; then
    echo "Error: Cargo.toml not found. Run this script from the repo root."
    exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Error: working tree is not clean. Commit or stash changes first."
    exit 1
fi

TODAY=$(date +%Y-%m-%d)

MANIFESTS=(Cargo.toml crates/logana-gui/Cargo.toml crates/logana-full/Cargo.toml)
for manifest in "${MANIFESTS[@]}"; do
    sed -i "0,/^version = /s/^version = \".*\"$/version = \"$VERSION\"/" "$manifest"
done

cargo update --workspace

cargo run --bin schema -- config 2>/dev/null > schema/config.schema.json
cargo run --bin schema -- custom-schema 2>/dev/null > schema/custom-schema.schema.json

sed -i "s|^## \[Unreleased\]$|## [Unreleased]\n\n## [$VERSION] - $TODAY|" CHANGELOG.md

git add "${MANIFESTS[@]}" Cargo.lock CHANGELOG.md schema/config.schema.json schema/custom-schema.schema.json
git commit -m "version v$VERSION"
git tag "v$VERSION"

echo ""
echo "Release v$VERSION prepared. To publish, run:"
echo "  git push && git push origin v$VERSION"
