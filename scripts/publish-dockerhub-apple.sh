#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="${VERSION:-$(git describe --tags --match 'v*' --abbrev=0 2>/dev/null || echo dev)}"
BUILD_DATE="${BUILD_DATE:-$(date -u +%Y%m%d)}"
VCS_REF="${VCS_REF:-$(git rev-parse --short=7 HEAD 2>/dev/null || echo nogithash)}"
IMAGE_REPO="${IMAGE_REPO:-docker.io/jasonxtt/mosdns-t}"
PUSH_LATEST="${PUSH_LATEST:-0}"
KEEP_ARCH_TAGS="${KEEP_ARCH_TAGS:-0}"

VERSION_TAG="${IMAGE_REPO}:${VERSION}"
AMD64_TAG="${IMAGE_REPO}:${VERSION}-amd64"
ARM64_TAG="${IMAGE_REPO}:${VERSION}-arm64"

docker_imagetools() {
  DOCKER_HOST=unix:///tmp/does-not-exist.sock docker buildx imagetools "$@"
}

dockerhub_delete_tag() {
  local full_tag="$1"
  python3 - "$full_tag" <<'PY'
import base64
import json
import os
import re
import sys
import urllib.error
import urllib.request

full_tag = sys.argv[1]
m = re.fullmatch(r"(?:docker\.io/)?([^/]+)/([^:]+):(.+)", full_tag)
if not m:
    print(f"skip deleting non-dockerhub tag: {full_tag}", file=sys.stderr)
    sys.exit(0)
namespace, repo, tag = m.groups()

cfg_paths = [
    os.path.expanduser("~/.docker/config.json"),
    "/Users/tom/.docker-container/config.json",
]
cred = None
for path in cfg_paths:
    if not os.path.exists(path):
        continue
    with open(path) as f:
        data = json.load(f)
    cred = data.get("auths", {}).get("https://index.docker.io/v1/")
    if cred:
        break
if not cred:
    raise SystemExit("docker hub credentials not found")

if "username" in cred and "password" in cred:
    username = cred["username"]
    password = cred["password"]
elif "auth" in cred:
    username, password = base64.b64decode(cred["auth"]).decode().split(":", 1)
else:
    raise SystemExit("unsupported docker hub credential format")

login_req = urllib.request.Request(
    "https://hub.docker.com/v2/users/login",
    data=json.dumps({"username": username, "password": password}).encode(),
    headers={"Content-Type": "application/json"},
)
with urllib.request.urlopen(login_req) as r:
    token = json.load(r)["token"]

delete_req = urllib.request.Request(
    f"https://hub.docker.com/v2/namespaces/{namespace}/repositories/{repo}/tags/{tag}",
    headers={"Authorization": f"JWT {token}"},
    method="DELETE",
)
try:
    with urllib.request.urlopen(delete_req) as r:
        print(f"deleted {full_tag}: {r.status}")
except urllib.error.HTTPError as e:
    if e.code == 404:
        print(f"tag already absent: {full_tag}")
    else:
        body = e.read().decode()
        raise SystemExit(f"failed deleting {full_tag}: HTTP {e.code} {body}")
PY
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

if [ "${KEEP_ARCH_TAGS}" != "1" ]; then
  dockerhub_delete_tag "${AMD64_TAG}"
  dockerhub_delete_tag "${ARM64_TAG}"
fi

echo "published ${VERSION_TAG}"
if [ "${PUSH_LATEST}" = "1" ]; then
  echo "published ${IMAGE_REPO}:latest"
fi
