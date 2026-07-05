#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="${VERSION:-$(git describe --tags --match 'v*' --abbrev=0 2>/dev/null || echo dev)}"
BUILD_DATE="${BUILD_DATE:-$(date -u +%Y%m%d)}"
VCS_REF="${VCS_REF:-$(git rev-parse --short=7 HEAD 2>/dev/null || echo nogithash)}"
IMAGE_REPO="${IMAGE_REPO:-docker.io/jasonxtt/mosdns-t}"
PUSH_LATEST="${PUSH_LATEST:-0}"

VERSION_TAG="${IMAGE_REPO}:${VERSION}"
AMD64_TAG="${IMAGE_REPO}:${VERSION}-amd64"
ARM64_TAG="${IMAGE_REPO}:${VERSION}-arm64"

docker_imagetools() {
  DOCKER_HOST=unix:///tmp/does-not-exist.sock docker buildx imagetools "$@"
}

export ROOT_DIR VERSION BUILD_DATE VCS_REF AMD64_TAG ARM64_TAG
"${ROOT_DIR}/scripts/with-apple-builder.sh" bash -lc '
  set -euo pipefail
  cd "$ROOT_DIR"

  container build \
    --platform linux/amd64 \
    -f Dockerfile_buildx \
    --build-arg VERSION="$VERSION" \
    --build-arg BUILD_DATE="$BUILD_DATE" \
    --build-arg VCS_REF="$VCS_REF" \
    -t "$AMD64_TAG" \
    .

  container build \
    --platform linux/arm64 \
    -f Dockerfile_buildx \
    --build-arg VERSION="$VERSION" \
    --build-arg BUILD_DATE="$BUILD_DATE" \
    --build-arg VCS_REF="$VCS_REF" \
    -t "$ARM64_TAG" \
    .

  container image push "$AMD64_TAG"
  container image push "$ARM64_TAG"
'

docker_imagetools create -t "${VERSION_TAG}" "${AMD64_TAG}" "${ARM64_TAG}"
docker_imagetools inspect "${VERSION_TAG}"

if [ "${PUSH_LATEST}" = "1" ]; then
  docker_imagetools create -t "${IMAGE_REPO}:latest" "${AMD64_TAG}" "${ARM64_TAG}"
  docker_imagetools inspect "${IMAGE_REPO}:latest"
fi

echo "published ${VERSION_TAG}"
if [ "${PUSH_LATEST}" = "1" ]; then
  echo "published ${IMAGE_REPO}:latest"
fi
