# syntax=docker/dockerfile:1

ARG NODE_VERSION=24
ARG DEBIAN_RELEASE=trixie
# Pin the multi-arch manifest so a moving tag cannot silently change the
# toolchain used for release artifacts.
ARG NODE_DIGEST=sha256:7f486cabe892bff8fef04c0f7da8dba84a560be3e4e2740b7a86afc57ebec315

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim@${NODE_DIGEST} AS base
ENV DEBIAN_FRONTEND=noninteractive \
    ELECTRON_CACHE=/root/.cache/electron \
    ELECTRON_BUILDER_CACHE=/root/.cache/electron-builder \
    YARN_CACHE_FOLDER=/root/.cache/yarn \
    COREPACK_HOME=/opt/corepack
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
    && yarn global add node-gyp@11 --prefix /usr/local
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
RUN yarn patch-package

# The local toolchain image used by every `make yarn` target. It adds the
# runtime libraries and Electron binary needed for `make dev`, while the
# release build below stays lean.
FROM deps AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      libgtk-3-0t64 \
      libnss3 \
      libasound2t64 \
      libgbm1 \
      libegl1 \
      libgles2 \
      libvulkan1 \
      mesa-vulkan-drivers \
      libxss1 \
      libxtst6 \
      libx11-xcb1 \
      libdrm2 \
      libnotify4 \
      libatspi2.0-0t64 \
      libsecret-1-0 \
      libpipewire-0.3-0 \
      gstreamer1.0-plugins-base \
      gstreamer1.0-plugins-good \
      gstreamer1.0-plugins-bad \
      gstreamer1.0-libav \
      gstreamer1.0-pipewire \
      xvfb \
    && rm -rf /var/lib/apt/lists/*
RUN --mount=type=cache,target=/root/.cache/electron \
    node node_modules/electron/install.js
# The Rust engine needs libxkbcommon-x11 at runtime for the X11 fallback
# (dlopened by winit's xkbcommon-dl), and libpipewire-0.3 for the native
# system-audio monitor capture (`modv-media` links it), which the base image
# does not ship.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libxkbcommon-x11-0 \
    && rm -rf /var/lib/apt/lists/*

# --- Rust engine (docs/rust-hybrid.md) -------------------------------------
# Host has no Rust toolchain, so the engine is built entirely in Docker,
# like every other yarn step in this file. The `rust` stage is also what
# `make engine*` uses for day-to-day iteration with the repo bind-mounted.
FROM base AS rust
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      libvulkan-dev \
      libasound2-dev \
      libudev-dev \
      libpipewire-0.3-dev \
      libgstreamer1.0-dev \
      libgstreamer-plugins-base1.0-dev \
      gstreamer1.0-plugins-base \
      gstreamer1.0-plugins-good \
      libclang-dev \
      libxkbcommon-dev \
      libgl1-mesa-dev \
      libwayland-dev \
      libx11-dev \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path \
    && rustup component add rustfmt clippy

# lavapipe (Mesa's software Vulkan) lets headless GPU tests and benchmarks
# run without a display or a real adapter.
RUN apt-get update \
    && apt-get install -y --no-install-recommends mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*
# The engine needs libxkbcommon-x11 for the X11 fallback at runtime
# (dlopened by winit's xkbcommon-dl).
RUN apt-get update \
    && apt-get install -y --no-install-recommends libxkbcommon-x11-0 \
    && rm -rf /var/lib/apt/lists/*

# Produces the engine binary copied into the packaging stages below. Kept as a
# separate stage so the layer is cached until crates/ actually changes.
FROM rust AS engine-build
WORKDIR /engine
COPY crates/ crates/
RUN cargo build --release --manifest-path crates/Cargo.toml

FROM deps AS build
ARG APP_VERSION
COPY . .
# electron-builder's extraResources picks the engine up from this path (see
# electron-builder.yml). Built in engine-build so the local yarn build does
# not need a host Rust toolchain.
COPY --from=engine-build /engine/crates/target/release/modv-engine ./crates/target/release/modv-engine
RUN if [ -n "$APP_VERSION" ]; then \
      yarn version --new-version "$APP_VERSION" --no-git-tag-version; \
    fi
RUN yarn build

FROM build AS package
# Default to both installers so CI/release behaviour is unchanged. Local
# AppImage-only builds override this with `make appimage`.
ARG PACKAGE_TARGETS
RUN --mount=type=cache,target=/root/.cache/electron \
    --mount=type=cache,target=/root/.cache/electron-builder \
    yarn electron-builder --linux ${PACKAGE_TARGETS:-AppImage deb} --publish never

FROM scratch AS appimage
COPY --from=package /app/dist/*.AppImage /

FROM scratch AS deb
COPY --from=package /app/dist/*.deb /

FROM scratch AS artifacts
COPY --from=appimage / /
COPY --from=deb / /
