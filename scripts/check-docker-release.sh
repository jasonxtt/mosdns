#!/usr/bin/env bash
set -euo pipefail

RELEASE_TAG="${1:?release tag is required}"
SOURCE_REF="${2:-HEAD}"
MAIN_REF="${3:-refs/remotes/origin/main}"

if [[ ! "$RELEASE_TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Docker releases require a stable vX.Y.Z tag: $RELEASE_TAG" >&2
  exit 1
fi

if ! git merge-base --is-ancestor "$SOURCE_REF" "$MAIN_REF"; then
  echo "Docker release source must be a commit on main: $SOURCE_REF" >&2
  exit 1
fi
