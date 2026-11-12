APP_VERSION ?= $(shell sed -n 's/^  "version": "\([^"]*\)".*/\1/p' package.json | head -n 1)
DOCKER_BUILD_ARGS ?=
DIST_DIR ?= dist

DOCKER ?= docker
TOOLCHAIN_IMAGE ?= modv-toolchain:local
NODE_MODULES_VOLUME ?= modv-node-modules
HOST_UID ?= $(shell id -u)
HOST_GID ?= $(shell id -g)
DOCKER_RUN_ARGS ?=
XAUTHORITY ?= $(HOME)/.Xauthority
PULSE_SOCKET ?= $(XDG_RUNTIME_DIR)/pulse/native
WAYLAND_DISPLAY ?= wayland-0
WAYLAND_SOCKET ?= $(XDG_RUNTIME_DIR)/$(WAYLAND_DISPLAY)
# Containers run as the host UID/GID and do not inherit supplementary groups,
# so the GPU device nodes stay unreadable unless their GIDs are added. Without
# this Chromium's GPU process aborts under load and takes the renderers with it.
DRI_GIDS ?= $(shell stat -c '%g' /dev/dri/card0 2>/dev/null) $(shell stat -c '%g' /dev/dri/renderD128 2>/dev/null)
DRI_GROUPS ?= $(foreach gid,$(sort $(DRI_GIDS)),--group-add $(gid))
DRI_ARGS ?= $(if $(wildcard /dev/dri/renderD* /dev/dri/card*),--device /dev/dri $(DRI_GROUPS))

# Prefer a Wayland socket when the host has one, otherwise fall back to X11.
DISPLAY_BACKEND ?= auto
ifeq ($(DISPLAY_BACKEND),auto)
  ifneq ($(wildcard $(WAYLAND_SOCKET)),)
    DISPLAY_BACKEND := wayland
  else
    DISPLAY_BACKEND := x11
  endif
endif

# Electron 39+ removed ELECTRON_OZONE_PLATFORM_HINT and selects the Ozone
# platform from XDG_SESSION_TYPE (defaulting to auto), so forward that instead.
ifeq ($(DISPLAY_BACKEND),wayland)
  GUI_ENV := -e XDG_RUNTIME_DIR="$(XDG_RUNTIME_DIR)" \
	-e WAYLAND_DISPLAY="$(WAYLAND_DISPLAY)" \
	-e XDG_SESSION_TYPE=wayland
  GUI_MOUNTS := -v "$(XDG_RUNTIME_DIR)":"$(XDG_RUNTIME_DIR)"
  GUI_AUDIO :=
else
  GUI_ENV := -e DISPLAY="$(DISPLAY)" -e XDG_SESSION_TYPE=x11
  GUI_MOUNTS := -v /tmp/.X11-unix:/tmp/.X11-unix \
	$(if $(wildcard $(XAUTHORITY)),-e XAUTHORITY=/tmp/.Xauthority -v "$(XAUTHORITY):/tmp/.Xauthority:ro")
  GUI_AUDIO := $(if $(wildcard $(PULSE_SOCKET)),-e PULSE_SERVER=unix:/tmp/pulse-native -v "$(PULSE_SOCKET):/tmp/pulse-native")
endif

GUI_RUN_ARGS := --net=host --shm-size=1g --ulimit core=0 $(DRI_ARGS) $(GUI_ENV) $(GUI_MOUNTS) $(GUI_AUDIO)

# E2E always runs through XWayland. Chromium cannot initialise Vulkan on Ozone
# Wayland ("'--ozone-platform=wayland' is not compatible with Vulkan"), and the
# GPU process crashes when a WebGPU scene runs under native Wayland. Running the
# whole suite through X11 keeps the WebGPU and non-WebGPU specs in one pass.
X11_GUI_ENV := -e DISPLAY="$(DISPLAY)" -e XDG_SESSION_TYPE=x11
X11_GUI_MOUNTS := -v /tmp/.X11-unix:/tmp/.X11-unix \
	$(if $(wildcard $(XAUTHORITY)),-e XAUTHORITY=/tmp/.Xauthority -v "$(XAUTHORITY):/tmp/.Xauthority:ro")
X11_GUI_AUDIO := $(if $(wildcard $(PULSE_SOCKET)),-e PULSE_SERVER=unix:/tmp/pulse-native -v "$(PULSE_SOCKET):/tmp/pulse-native")
GUI_RUN_ARGS_X11 := --net=host --shm-size=1g --ulimit core=0 $(DRI_ARGS) $(X11_GUI_ENV) $(X11_GUI_MOUNTS) $(X11_GUI_AUDIO)

.DEFAULT_GOAL := build
RUST_IMAGE ?= modv-rust:local
CARGO_VOLUME ?= modv-cargo

# Building the engine and the installers needs several GB of BuildKit cache.
# Fail before a build starts when the filesystem is nearly full instead of
# letting BuildKit fill the disk (which breaks every later command).
MIN_FREE_GB ?= 20

.DEFAULT_GOAL := build
.PHONY: build appimage clean docker-deps docker-node-modules docker-reset install dev lint format test-e2e test-e2e-all test-unit yarn shell rave-reencode docker-rust docker-cargo-volume engine engine-test engine-lint engine-run disk-guard disk-clean

disk-guard:
	@free=$$(df -Pk "$(CURDIR)" | awk 'NR==2 {print int($$4/1048576)}'); \
	if [ "$$free" -lt "$(MIN_FREE_GB)" ]; then \
		echo "error: only $${free} GB free on the build filesystem (need $(MIN_FREE_GB) GB)."; \
		echo "       run 'make disk-clean' (docker builder prune) and retry."; \
		exit 1; \
	fi

disk-clean:
	-$(DOCKER) builder prune -af
	-$(DOCKER) image prune -f
	-$(DOCKER) container prune -f

build: disk-guard
	mkdir -p "$(DIST_DIR)"
	find "$(DIST_DIR)" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
	docker buildx build \
		--build-arg "APP_VERSION=$(APP_VERSION)" \
		--target artifacts \
		--output type=local,dest="$(DIST_DIR)" \
		$(DOCKER_BUILD_ARGS) .

# AppImage-only variant of `build`. The deb packaging step is by far the
# slowest part (~70s vs ~2s for the AppImage), so this skips it for local runs.
appimage: disk-guard
	mkdir -p "$(DIST_DIR)"
	find "$(DIST_DIR)" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
	docker buildx build \
		--build-arg "APP_VERSION=$(APP_VERSION)" \
		--build-arg "PACKAGE_TARGETS=AppImage" \
		--target appimage \
		--output type=local,dest="$(DIST_DIR)" \
		$(DOCKER_BUILD_ARGS) .

clean:
	rm -rf "$(DIST_DIR)"

docker-deps: disk-guard
	$(DOCKER) build --target toolchain -t "$(TOOLCHAIN_IMAGE)" .

# Seed the node_modules volume from the toolchain image the first time it is
# used, then hand ownership to the host user so lint/build can write back to the
# bind-mounted repository.
docker-node-modules: docker-deps
	@if $(DOCKER) volume inspect "$(NODE_MODULES_VOLUME)" >/dev/null 2>&1; then \
		:; \
	else \
		echo "Seeding $(NODE_MODULES_VOLUME) from $(TOOLCHAIN_IMAGE)"; \
		$(DOCKER) run --rm -v "$(NODE_MODULES_VOLUME):/app/node_modules" "$(TOOLCHAIN_IMAGE)" true; \
		$(DOCKER) run --rm -v "$(NODE_MODULES_VOLUME):/app/node_modules" "$(TOOLCHAIN_IMAGE)" \
			chown -R "$(HOST_UID):$(HOST_GID)" /app/node_modules; \
	fi

docker-reset:
	-$(DOCKER) volume rm "$(NODE_MODULES_VOLUME)"

define run-yarn
	@mkdir -p "$(CURDIR)/node_modules"
	$(DOCKER) run --rm \
		--user "$(HOST_UID):$(HOST_GID)" \
		-e HOME=/tmp \
		-e YARN_CACHE_FOLDER=/tmp/yarn-cache \
		-v "$(CURDIR)":/app \
		-v "$(NODE_MODULES_VOLUME)":/app/node_modules \
		-w /app \
		$(DOCKER_RUN_ARGS) \
		"$(TOOLCHAIN_IMAGE)" \
		yarn $(1)
endef

install: docker-deps
	-$(DOCKER) volume rm "$(NODE_MODULES_VOLUME)" >/dev/null
	@echo "Seeding $(NODE_MODULES_VOLUME) from $(TOOLCHAIN_IMAGE)"
	$(DOCKER) run --rm -v "$(NODE_MODULES_VOLUME):/app/node_modules" "$(TOOLCHAIN_IMAGE)" true
	$(DOCKER) run --rm -v "$(NODE_MODULES_VOLUME):/app/node_modules" "$(TOOLCHAIN_IMAGE)" \
		chown -R "$(HOST_UID):$(HOST_GID)" /app/node_modules

dev: DOCKER_RUN_ARGS += $(GUI_RUN_ARGS) -e NO_SANDBOX=1
dev: docker-node-modules
	$(call run-yarn,dev $(ARGS))

lint: docker-node-modules
	$(call run-yarn,lint)

format: docker-node-modules
	$(call run-yarn,format)

test-e2e: DOCKER_RUN_ARGS += $(GUI_RUN_ARGS_X11) -e MODV_E2E_NO_SANDBOX=1
test-e2e: docker-node-modules
	$(call run-yarn,test:e2e $(ARGS))

# Backwards-compatible alias: the WebGPU and non-WebGPU specs now run together
# in the one X11 pass.
test-e2e-all: test-e2e

test-unit: docker-node-modules
	$(call run-yarn,test:unit $(ARGS))

rave-reencode: docker-node-modules
	$(call run-yarn,node tools/rave-reencode.mjs $(ARGS))

yarn: docker-node-modules
	$(call run-yarn,$(ARGS))

docker-rust: disk-guard
	$(DOCKER) build --target rust -t "$(RUST_IMAGE)" .

docker-cargo-volume: docker-rust
	@$(DOCKER) volume inspect "$(CARGO_VOLUME)" >/dev/null 2>&1 || $(DOCKER) volume create "$(CARGO_VOLUME)" >/dev/null
	$(DOCKER) run --rm -v "$(CARGO_VOLUME)":/cargo "$(RUST_IMAGE)" chown -R "$(HOST_UID):$(HOST_GID)" /cargo

define run-cargo
	$(DOCKER) run --rm \
		--ulimit core=0 \
		--user "$(HOST_UID):$(HOST_GID)" \
		-e HOME=/tmp \
		-e CARGO_HOME=/cargo \
		-v "$(CURDIR)":/app \
		-v "$(CARGO_VOLUME)":/cargo \
		-w /app/crates \
		$(DOCKER_RUN_ARGS) \
		"$(RUST_IMAGE)" \
		cargo $(1)
endef

engine: docker-cargo-volume
	$(call run-cargo,build --release)

engine-test: docker-cargo-volume
	$(call run-cargo,test)

engine-lint: docker-cargo-volume
	$(call run-cargo,clippy --all-targets -- -D warnings)
	$(call run-cargo,fmt --check)

# Run the engine's native window against the host session (Wayland preferred,
# X11 fallback) with GPU forwarding, mirroring `make dev`.
engine-run: DOCKER_RUN_ARGS += $(GUI_RUN_ARGS)
engine-run: docker-cargo-volume
	$(call run-cargo,run --release -p modv-app -- $(ARGS))

shell: docker-node-modules
	@mkdir -p "$(CURDIR)/node_modules"
	$(DOCKER) run --rm -it --user "$(HOST_UID):$(HOST_GID)" -e HOME=/tmp \
		-e YARN_CACHE_FOLDER=/tmp/yarn-cache \
		-v "$(CURDIR)":/app -v "$(NODE_MODULES_VOLUME)":/app/node_modules -w /app \
		$(DOCKER_RUN_ARGS) "$(TOOLCHAIN_IMAGE)" sh
