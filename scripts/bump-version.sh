#!/usr/bin/env bash
# Bump the version in Cargo.toml. Prints the new version to stdout.
# Usage: ./scripts/bump-version.sh [patch|minor|major] [--dry-run]
set -euo pipefail

bump="patch"
dry_run=0
for arg; do
    case "$arg" in
        --dry-run) dry_run=1 ;;
        patch|minor|major) bump="$arg" ;;
        *) echo "unknown arg: $arg (expected patch|minor|major or --dry-run)" >&2; exit 1 ;;
    esac
done

cargo_toml="$(dirname "$0")/../Cargo.toml"
current=$(grep -m1 '^version = ' "$cargo_toml" | sed -E 's/^version = "(.*)"$/\1/')
IFS=. read -r maj min pat <<<"$current"

case "$bump" in
    patch) pat=$((pat + 1)) ;;
    minor) min=$((min + 1)); pat=0 ;;
    major) maj=$((maj + 1)); min=0; pat=0 ;;
esac

new="$maj.$min.$pat"
if [[ $dry_run -eq 0 ]]; then
    sed -i -E "s/^version = \"$current\"$/version = \"$new\"/" "$cargo_toml"
fi
echo "$new"
