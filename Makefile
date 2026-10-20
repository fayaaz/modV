APP_VERSION ?= $(shell node -p "require('./package.json').version")
DOCKER_BUILD_ARGS ?=
DIST_DIR ?= dist

.DEFAULT_GOAL := build
.PHONY: build clean

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
