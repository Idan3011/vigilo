#!/usr/bin/env bash
set -e

BUMP="${1:-patch}"

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Working tree is not clean. Commit or stash changes first."
  exit 1
fi

CURRENT=$(grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)
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

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Tag $TAG already exists."
  exit 1
fi

echo "  ${CURRENT} → ${NEXT}"

sed -i.bak "s/^version = \"${CURRENT}\"/version = \"${NEXT}\"/" Cargo.toml
rm -f Cargo.toml.bak

cargo check --quiet || { echo "cargo check failed after version bump"; exit 1; }

git add Cargo.toml Cargo.lock
git commit -m "Release ${TAG}"
git tag "$TAG"

echo "  Tagged ${TAG}"
echo "  Run: git push origin main ${TAG}"
