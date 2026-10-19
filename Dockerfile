# syntax=docker/dockerfile:1

ARG NODE_VERSION=24
ARG DEBIAN_RELEASE=trixie

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS base
ENV DEBIAN_FRONTEND=noninteractive \
    ELECTRON_CACHE=/root/.cache/electron \
    ELECTRON_BUILDER_CACHE=/root/.cache/electron-builder \
    YARN_CACHE_FOLDER=/root/.cache/yarn
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      wget \
      git \
      python3 \
      make \
      g++ \
      pkg-config \
      file \
      fakeroot \
      xz-utils \
      libarchive-tools \
    && rm -rf /var/lib/apt/lists/*
RUN corepack enable \
    && corepack prepare yarn@1.22.22 --activate \
    && npm install --global node-gyp@11
WORKDIR /app

FROM base AS deps
COPY package.json yarn.lock ./
COPY patches ./patches
RUN --mount=type=cache,target=/root/.cache/yarn \
    --mount=type=cache,target=/root/.cache/electron \
    --mount=type=cache,target=/root/.cache/electron-builder \
    yarn install --frozen-lockfile --ignore-scripts --non-interactive
# The vendored grandiose fork pins the NDI 4 soname (libndi.so.5.1.1) in its
# binding.gyp, while the NDI 5 SDK it downloads ships libndi.so.5.6.1. Alias
# the versioned library so the copy step and linker resolve it, then build the
# addon against the Electron headers the packaged app actually runs on.
RUN cd node_modules/grandiose \
    && node ndi.js \
    && (cd ndi/lib/lnx-x64 \
        && cp -L "$(ls libndi.so.*.*.* | head -n1)" libndi.so.5.1.1 \
        && ln -sf libndi.so.5.1.1 libndi.so.5 \
        && ln -sf libndi.so.5.1.1 libndi.so \
        && ls -l) \
    && node-gyp rebuild \
        --target="$(node -p "require('/app/node_modules/electron/package.json').version")" \
        --arch=x64 \
        --dist-url=https://electronjs.org/headers
RUN npx patch-package

FROM deps AS build
ARG APP_VERSION
COPY . .
RUN if [ -n "$APP_VERSION" ]; then \
      npm version "$APP_VERSION" --no-git-tag-version --allow-same-version; \
    fi
RUN npm run build

FROM build AS package
RUN npx electron-builder --linux AppImage deb --publish never

FROM scratch AS appimage
COPY --from=package /app/dist/*.AppImage /

FROM scratch AS deb
COPY --from=package /app/dist/*.deb /

FROM scratch AS artifacts
COPY --from=appimage / /
COPY --from=deb / /
