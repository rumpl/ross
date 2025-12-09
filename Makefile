.PHONY: build-image dev dev-watch dev-build dev-test dev-clippy dev-shell dev-run build test clippy fmt clean

IMAGE_NAME := ross-dev
DATA_DIR := /tmp/ross-data

# Build the Docker image with compiled binaries
build-image:
	docker build -f Dockerfile.dev -t $(IMAGE_NAME) .

# Run the daemon (requires build-image first)
dev-run: build-image
	docker run -it --rm \
		--privileged \
		-v $(DATA_DIR):/tmp/ross \
		-p 50051:50051 \
		$(IMAGE_NAME) daemon

# Development targets (mount source code for iteration)
dev: build-image
	docker run -it --rm -v $(PWD):/app -p 50051:50051 $(IMAGE_NAME) shell

dev-shell: build-image
	docker run -it --rm --privileged -v $(DATA_DIR):/tmp/ross --network host $(IMAGE_NAME) shell

# Build/test/lint using mounted source (for development iteration)
dev-build:
	docker build -f Dockerfile.dev -t $(IMAGE_NAME) .

dev-test:
	docker run --rm -v $(PWD):/app -w /app rust:alpine sh -c '\
		apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconf protoc && \
		cargo test --workspace --lib'

dev-clippy:
	docker run --rm -v $(PWD):/app -w /app rust:alpine sh -c '\
		apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconf protoc && \
		rustup component add clippy && \
		cargo clippy --workspace -- -D warnings'

# Local targets (for macOS/non-Linux, limited functionality)
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
	rm -rf $(DATA_DIR)
