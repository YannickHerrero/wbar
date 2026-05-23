#!/usr/bin/env bash
# Cut a release: bump version, commit, tag, push.
# Usage:
#   ./scripts/deploy.sh                  -> interactive (prompts for bump + confirm)
#   ./scripts/deploy.sh patch|minor|major -> non-interactive
set -euo pipefail

bump="${1:-}"
here="$(dirname "$0")"

if [[ -z "$bump" ]]; then
    if [[ ! -t 0 ]]; then
        echo "ERROR: no bump type given and stdin is not a terminal." >&2
        echo "Run: make deploy BUMP=patch|minor|major" >&2
        exit 1
    fi
    read -rp "Bump type [patch/minor/major] (default: patch): " bump
    bump="${bump:-patch}"
fi

case "$bump" in
    patch|minor|major) ;;
    *) echo "ERROR: invalid bump type: $bump" >&2; exit 1 ;;
esac

"$here/check-clean-tree.sh"

current=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/^version = "(.*)"$/\1/')
new=$("$here/bump-version.sh" "$bump" --dry-run)

echo
echo "  Current: v$current"
echo "  Bump:    $bump"
echo "  New:     v$new"
echo

if [[ -t 0 ]]; then
    read -rp "Proceed? [Y/n] " ans
    case "${ans:-y}" in
        y|Y|yes|YES) ;;
        *) echo "Aborted."; exit 0 ;;
    esac
fi

"$here/bump-version.sh" "$bump" >/dev/null
cargo check --target x86_64-pc-windows-gnu --offline
git add Cargo.toml Cargo.lock
git commit -m "release: v$new"
git tag -a "v$new" -m "v$new"
git push origin main
git push origin "v$new"
echo "Pushed v$new — CI build should start shortly."
