#!/usr/bin/env bash
set -e

BUMP="${1:-patch}"

CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$BUMP" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
  *)
    echo "Usage: $0 [major|minor|patch]"
    exit 1
    ;;
esac

NEXT="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEXT}"

echo "  ${CURRENT} → ${NEXT}"

sed -i "s/^version = \"${CURRENT}\"/version = \"${NEXT}\"/" Cargo.toml
cargo check --quiet 2>/dev/null

git add Cargo.toml Cargo.lock
git commit -m "Release ${TAG}"
git tag "$TAG"

echo "  Tagged ${TAG}"
echo "  Run: git push origin main ${TAG}"
