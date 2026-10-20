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
DRI_ARGS ?= $(if $(wildcard /dev/dri/renderD* /dev/dri/card*),--device /dev/dri)

# Prefer a Wayland socket when the host has one, otherwise fall back to X11.
DISPLAY_BACKEND ?= auto
ifeq ($(DISPLAY_BACKEND),auto)
  ifneq ($(wildcard $(WAYLAND_SOCKET)),)
    DISPLAY_BACKEND := wayland
  else
    DISPLAY_BACKEND := x11
  endif
endif

ifeq ($(DISPLAY_BACKEND),wayland)
  GUI_ENV := -e XDG_RUNTIME_DIR="$(XDG_RUNTIME_DIR)" \
	-e WAYLAND_DISPLAY="$(WAYLAND_DISPLAY)" \
	-e ELECTRON_OZONE_PLATFORM_HINT=wayland
  GUI_MOUNTS := -v "$(XDG_RUNTIME_DIR)":"$(XDG_RUNTIME_DIR)"
  GUI_AUDIO :=
else
  GUI_ENV := -e DISPLAY="$(DISPLAY)" -e ELECTRON_OZONE_PLATFORM_HINT=x11
  GUI_MOUNTS := -v /tmp/.X11-unix:/tmp/.X11-unix \
	$(if $(wildcard $(XAUTHORITY)),-e XAUTHORITY=/tmp/.Xauthority -v "$(XAUTHORITY):/tmp/.Xauthority:ro")
  GUI_AUDIO := $(if $(wildcard $(PULSE_SOCKET)),-e PULSE_SERVER=unix:/tmp/pulse-native -v "$(PULSE_SOCKET):/tmp/pulse-native")
endif

GUI_RUN_ARGS := --net=host --shm-size=1g $(DRI_ARGS) $(GUI_ENV) $(GUI_MOUNTS) $(GUI_AUDIO)

.DEFAULT_GOAL := build
.PHONY: build clean docker-deps docker-node-modules docker-reset install dev lint format test-e2e yarn shell

build:
	mkdir -p "$(DIST_DIR)"
	find "$(DIST_DIR)" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
	docker buildx build \
		--build-arg "APP_VERSION=$(APP_VERSION)" \
		--target artifacts \
		--output type=local,dest="$(DIST_DIR)" \
		$(DOCKER_BUILD_ARGS) .

clean:
	rm -rf "$(DIST_DIR)"

docker-deps:
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

test-e2e: DOCKER_RUN_ARGS += $(GUI_RUN_ARGS) -e MODV_E2E_NO_SANDBOX=1
test-e2e: docker-node-modules
	$(call run-yarn,test:e2e $(ARGS))

yarn: docker-node-modules
	$(call run-yarn,$(ARGS))

shell: docker-node-modules
	@mkdir -p "$(CURDIR)/node_modules"
	$(DOCKER) run --rm -it --user "$(HOST_UID):$(HOST_GID)" -e HOME=/tmp \
		-e YARN_CACHE_FOLDER=/tmp/yarn-cache \
		-v "$(CURDIR)":/app -v "$(NODE_MODULES_VOLUME)":/app/node_modules -w /app \
		$(DOCKER_RUN_ARGS) "$(TOOLCHAIN_IMAGE)" sh
