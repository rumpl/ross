# Ross Project Guide

Ross is a container runtime system written in Rust, structured as a multi-crate workspace. This document describes the project architecture and provides guidance for working with the codebase.

## Project Structure

```
ross/
├── cli/          # Command-line interface (ross-cli)
├── container/    # Container management logic (ross-container)
├── core/         # gRPC protocol definitions (ross-core)
├── daemon/       # gRPC server (ross-daemon)
├── image/        # Image management logic (ross-image)
├── proto/        # Protocol buffer definitions
├── remote/       # Registry client (ross-remote)
└── store/        # Content-addressable storage (ross-store)
```

## Crate Descriptions

### `ross-core` (core/)

Generated gRPC types and service traits from protocol buffer definitions. This crate uses `tonic-build` to compile `.proto` files from the `proto/` directory.

- **Purpose**: Shared protocol definitions for client-server communication
- **Key exports**: gRPC service traits, request/response types
- **Dependencies**: prost, tonic

### `ross-container` (container/)

Core container management logic, completely decoupled from gRPC.

- **Purpose**: Container lifecycle management (create, start, stop, remove, etc.)
- **Key exports**: `ContainerService`, container types, `ContainerError`
- **Note**: This crate has no knowledge of gRPC or protocol buffers

Key files:
- `service.rs` - Main `ContainerService` implementation
- `types.rs` - Domain types (Container, ContainerConfig, HostConfig, etc.)
- `error.rs` - Error type definitions

### `ross-image` (image/)

Core image management logic, completely decoupled from gRPC.

- **Purpose**: Image operations (pull, push, build, list, tag, etc.)
- **Key exports**: `ImageService`, image types, `ImageError`
- **Dependencies**: ross-remote, ross-store
- **Note**: This crate has no knowledge of gRPC or protocol buffers

Key files:
- `service.rs` - Main `ImageService` implementation with pull/push logic
- `types.rs` - Domain types (Image, PullProgress, BuildParams, etc.)
- `error.rs` - Error type definitions

### `ross-daemon` (daemon/)

The gRPC server that exposes container and image services over the network.

- **Purpose**: Thin adapter layer between gRPC and core services
- **Responsibilities**:
  - Request validation
  - Type conversion (gRPC ↔ domain types)
  - Delegation to core services
- **Dependencies**: ross-core, ross-container, ross-image, ross-store

Key files:
- `main.rs` - Server startup and configuration
- `services/container.rs` - gRPC adapter for ContainerService
- `services/image.rs` - gRPC adapter for ImageService
- `services/ross.rs` - Health check service

### `ross-cli` (cli/)

Command-line interface for interacting with the daemon.

- **Purpose**: User-facing CLI tool
- **Dependencies**: ross-core (for gRPC client)
- **Commands**: container operations, image operations, health checks

### `ross-remote` (remote/)

Registry client for interacting with container registries (Docker Hub, etc.).

- **Purpose**: Pull/push images from/to OCI-compliant registries
- **Key exports**: `RegistryClient`, `ImageReference`, manifest types
- **Handles**: Authentication, manifest resolution, blob downloads

Key files:
- `client.rs` - HTTP client for registry API
- `reference.rs` - Image reference parsing (e.g., `docker.io/library/alpine:latest`)
- `types.rs` - OCI manifest and config types

### `ross-store` (store/)

Content-addressable storage for blobs and manifests.

- **Purpose**: Local storage of image layers, configs, and manifests
- **Key exports**: `FileSystemStore`, `Digest`, `StoreError`
- **Storage layout**: Blobs and manifests stored by digest

Key files:
- `storage.rs` - FileSystemStore implementation
- `error.rs` - Storage error types

### Protocol Buffers (proto/)

gRPC service and message definitions.

- `ross.proto` - Health check service
- `container.proto` - Container service (create, start, stop, etc.)
- `image.proto` - Image service (pull, push, build, etc.)
- `store.proto` - Storage service definitions

## Architecture Principles

### Separation of Concerns

The architecture follows a clean separation:

1. **Core services** (`ross-container`, `ross-image`) contain business logic with no transport dependencies
2. **Daemon** (`ross-daemon`) is a thin adapter that only handles gRPC concerns
3. **Storage** (`ross-store`) and **registry** (`ross-remote`) are infrastructure concerns

### Type Conversion Pattern

The daemon converts between gRPC types and domain types using explicit conversion functions:

```rust
// gRPC → Domain
fn container_config_from_grpc(c: ross_core::ContainerConfig) -> ross_container::ContainerConfig

// Domain → gRPC  
fn container_to_grpc(c: ross_container::Container) -> ross_core::Container
```

This avoids orphan rule issues with `From` trait implementations.

## Development Commands

```bash
# Build the project
cargo build

# Run tests
cargo test

# Run linter
cargo clippy

# Format code
cargo fmt

# Run the daemon
cargo run -p ross-daemon -- start --port 50051

# Run the CLI
cargo run -p ross-cli -- health
```

## Adding New Features

### Adding a new container operation

1. Add the method to `ContainerService` in `container/src/service.rs`
2. Add any new types to `container/src/types.rs`
3. Add the gRPC endpoint in `daemon/src/services/container.rs`
4. Add conversion functions for new types
5. Update the proto file if needed and regenerate

### Adding a new image operation

1. Add the method to `ImageService` in `image/src/service.rs`
2. Add any new types to `image/src/types.rs`
3. Add the gRPC endpoint in `daemon/src/services/image.rs`
4. Add conversion functions for new types
5. Update the proto file if needed and regenerate

## Testing

- Unit tests go in the same file as the code using `#[cfg(test)]` modules
- Integration tests go in `tests/` directories within each crate
- Use `cargo test` to run all tests
- Use `cargo test -p ross-container` to test a specific crate
