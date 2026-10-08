#!/usr/bin/env bash
set -euo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-docker-release.sh"
TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT

git init --quiet --initial-branch=main "$TEST_DIR"
cd "$TEST_DIR"
git config user.name "Release guard test"
git config user.email "release-test@example.invalid"
git commit --quiet --allow-empty -m baseline
BASE="$(git rev-parse HEAD)"
git switch --quiet -c platform
git commit --quiet --allow-empty -m platform-only
PLATFORM="$(git rev-parse HEAD)"

"$GUARD" v0.7.4 "$BASE" main
for tag in preview-20261009 lite-v0.1.9 openwrt-v0.7.3 v0.7.4-rc1 v0.7 v01.7.4 'v0.7.4;false'; do
  if "$GUARD" "$tag" "$BASE" main >/dev/null 2>&1; then
    echo "Unexpectedly accepted tag: $tag" >&2
    exit 1
  fi
done
if "$GUARD" v0.7.4 "$PLATFORM" main >/dev/null 2>&1; then
  echo "Unexpectedly accepted a platform-only commit" >&2
  exit 1
fi
if "$GUARD" v0.7.4 "$BASE" missing-main >/dev/null 2>&1; then
  echo "Unexpectedly accepted an unavailable main ref" >&2
  exit 1
fi
echo "Docker release guard passed"
