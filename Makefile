APP_VERSION ?= $(shell node -p "require('./package.json').version")
DOCKER_BUILD_ARGS ?=

.DEFAULT_GOAL := build
.PHONY: build clean

build:
	rm -rf dist
	docker buildx build \
		--build-arg "APP_VERSION=$(APP_VERSION)" \
		--target artifacts \
		--output type=local,dest=dist \
		$(DOCKER_BUILD_ARGS) .

clean:
	rm -rf dist
