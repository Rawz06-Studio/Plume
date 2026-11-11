#!/usr/bin/env bash
# Bumps the version in Cargo.toml and Cargo.lock, then prints the new version.
# Usage: scripts/bump-version.sh patch|minor|major
set -euo pipefail
cd "$(dirname "$0")/.."

current=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
IFS=. read -r major minor patch <<< "${current%%-*}"

case "${1:-}" in
  major) next="$((major + 1)).0.0" ;;
  minor) next="$major.$((minor + 1)).0" ;;
  patch) next="$major.$minor.$((patch + 1))" ;;
  *) echo "usage: $0 patch|minor|major" >&2; exit 1 ;;
esac

# Only the first `version = ` line is the package version.
perl -0pi -e "s/^version = \"\Q$current\E\"/version = \"$next\"/m" Cargo.toml
# The lock file entry right after `name = "plume"`.
perl -0pi -e "s/(name = \"plume\"\nversion = )\"\Q$current\E\"/\$1\"$next\"/" Cargo.lock

echo "$next"
