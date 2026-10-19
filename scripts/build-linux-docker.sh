#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

version=${APP_VERSION:-$(node -p "require('./package.json').version")}

rm -rf dist
docker buildx build \
  --build-arg "APP_VERSION=$version" \
  --target artifacts \
  --output type=local,dest=dist \
  "$@" \
  .
