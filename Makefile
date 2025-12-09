.PHONY: dev dev-watch dev-build dev-test dev-clippy dev-shell build test clippy fmt clean

IMAGE_NAME := ross-dev

# Development container targets
dev: dev-image
	docker run -it --rm -v $(PWD):/app -p 50051:50051 $(IMAGE_NAME)

dev-watch: dev-image
	docker run -it --rm -v $(PWD):/app -p 50051:50051 $(IMAGE_NAME) watch

dev-build: dev-image
	docker run -it --rm -v $(PWD):/app $(IMAGE_NAME) build

dev-test: dev-image
	docker run -it --rm -v $(PWD):/app $(IMAGE_NAME) test

dev-clippy: dev-image
	docker run -it --rm -v $(PWD):/app $(IMAGE_NAME) clippy

dev-shell: dev-image
	docker run -it --rm -v $(PWD):/app $(IMAGE_NAME) shell

dev-image:
	docker build -f Dockerfile.dev -t $(IMAGE_NAME) .

# Local targets
build:
	cargo build --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --all

clean:
	cargo clean
