.PHONY: all build build-shim build-guest build-image dev dev-shell dev-run test clippy fmt clean

IMAGE_NAME := ross-dev
DATA_DIR := /tmp/ross-data
GUEST_TARGET := aarch64-unknown-linux-musl

# ============================================================================
# Main Build Targets
# ============================================================================

# Build everything for macOS with libkrun support
# This builds: 1) guest binary, 2) workspace, 3) shim with embedded guest
all: build-guest-docker build build-shim

# Build everything (host components)
build:
	cargo build --workspace

# Build with libkrun support (macOS only)
# Requires: brew install llvm libkrun
# The DYLD_FALLBACK_LIBRARY_PATH is needed for libclang used by bindgen
build-shim:
	DYLD_FALLBACK_LIBRARY_PATH=$$(brew --prefix llvm)/lib \
	LIBCLANG_PATH=$$(brew --prefix llvm)/lib \
	cargo build -p ross-shim --features libkrun

# Build guest init binary (requires cross-compilation toolchain for Linux)
# This binary runs inside the VM and must be compiled for Linux
build-guest:
	@echo "Building ross-init for Linux guest..."
	@echo "This requires the $(GUEST_TARGET) target installed."
	@echo "Install with: rustup target add $(GUEST_TARGET)"
	cd guest && cargo build --release --target $(GUEST_TARGET)
	@echo "Guest binary built at: guest/target/$(GUEST_TARGET)/release/ross-init"

# Build guest using Docker (no local cross toolchain required)
build-guest-docker:
	@echo "Building ross-init for Linux guest (aarch64) using Docker..."
	docker run --rm \
		--platform linux/arm64 \
		-v $(PWD)/guest:/app \
		-w /app \
		rust:alpine \
		sh -c 'apk add --no-cache musl-dev && cargo build --release'
	@echo "Guest binary built at: guest/target/release/ross-init"

# ============================================================================
# Docker-based Development (for Linux features like runc)
# ============================================================================

# Build the Docker image with compiled binaries
build-image:
	docker build -f Dockerfile.dev -t $(IMAGE_NAME) .

# Run the daemon in Docker
dev-run: build-image
	docker run -it --rm \
		--privileged \
		-v $(DATA_DIR):/tmp/ross \
		-p 50051:50051 \
		$(IMAGE_NAME) daemon

# Development shell in Docker
dev: build-image
	docker run -it --rm -v $(PWD):/app -p 50051:50051 $(IMAGE_NAME) shell

dev-shell: build-image
	docker run -it --rm --privileged -v $(DATA_DIR):/tmp/ross --network host $(IMAGE_NAME) shell

# ============================================================================
# Testing and Quality
# ============================================================================

test:
	cargo test --workspace

# Test in Docker (for Linux-specific tests)
dev-test:
	docker run --rm -v $(PWD):/app -w /app rust:alpine sh -c '\
		apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconf protoc && \
		cargo test --workspace --lib'

clippy:
	cargo clippy --workspace -- -D warnings

# Clippy in Docker
dev-clippy:
	docker run --rm -v $(PWD):/app -w /app rust:alpine sh -c '\
		apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconf protoc && \
		rustup component add clippy && \
		cargo clippy --workspace -- -D warnings'

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

# ============================================================================
# libkrun Interactive Container Testing (macOS)
# ============================================================================

# Prepare a test rootfs with ross-init installed
# This downloads alpine minirootfs and adds the ross-init binary
prepare-test-rootfs: build-guest-docker
	@echo "Preparing test rootfs..."
	@mkdir -p /tmp/ross-test-rootfs
	@curl -L https://dl-cdn.alpinelinux.org/alpine/v3.19/releases/aarch64/alpine-minirootfs-3.19.0-aarch64.tar.gz | tar -xz -C /tmp/ross-test-rootfs
	@cp guest/target/release/ross-init /tmp/ross-test-rootfs/ross-init
	@chmod +x /tmp/ross-test-rootfs/ross-init
	@echo "Test rootfs ready at /tmp/ross-test-rootfs"

# ============================================================================
# Clean
# ============================================================================

clean:
	cargo clean
	rm -rf $(DATA_DIR)
	rm -rf /tmp/ross-test-rootfs
	cd guest && cargo clean 2>/dev/null || true

# ============================================================================
# Help
# ============================================================================

help:
	@echo "Ross Container Runtime - Build Targets"
	@echo ""
	@echo "Quick Start (macOS with libkrun):"
	@echo "  make all         - Build everything (guest + host + shim with libkrun)"
	@echo ""
	@echo "Host Build:"
	@echo "  build            - Build all workspace crates"
	@echo "  build-shim       - Build shim with libkrun support (macOS)"
	@echo ""
	@echo "Guest Build (Linux VM init process):"
	@echo "  build-guest        - Build ross-init (requires cross toolchain)"
	@echo "  build-guest-docker - Build ross-init using Docker"
	@echo ""
	@echo "Docker Development:"
	@echo "  build-image      - Build Docker development image"
	@echo "  dev-run          - Run daemon in Docker"
	@echo "  dev-shell        - Open shell in Docker"
	@echo ""
	@echo "Testing:"
	@echo "  test             - Run tests locally"
	@echo "  dev-test         - Run tests in Docker"
	@echo "  prepare-test-rootfs - Prepare Alpine rootfs with ross-init"
	@echo ""
	@echo "Quality:"
	@echo "  clippy           - Run clippy locally"
	@echo "  fmt              - Format code"
	@echo "  fmt-check        - Check formatting"
	@echo ""
	@echo "Cleanup:"
	@echo "  clean            - Remove build artifacts"
