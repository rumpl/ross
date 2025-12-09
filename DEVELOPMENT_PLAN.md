# Ross - Development Plan

A Rust cargo workspace with core, cli, and daemon crates, featuring gRPC communication.

## Project Overview

**Project Name:** ross  
**Structure:** Cargo workspace with 3 crates  
**Communication:** gRPC (tonic/prost)  
**Runtime:** Async (tokio)  

## Architecture

```
ross/
├── Cargo.toml                 # Workspace root
├── Dockerfile                 # Multi-stage build for daemon
├── proto/
│   └── ross.proto             # gRPC service definitions
├── core/
│   ├── Cargo.toml
│   ├── build.rs               # Proto compilation
│   └── src/
│       └── lib.rs             # Shared types + generated proto code
├── cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs            # CLI entry point with clap
└── daemon/
    ├── Cargo.toml
    └── src/
        └── main.rs            # Daemon entry point with start command
```

## Dependencies

### Core Crate
- `prost` - Protocol buffer implementation
- `tonic` - gRPC framework
- `serde` + `serde_derive` - Serialization

### CLI Crate
- `clap` (with derive feature) - Command line parsing
- `tokio` - Async runtime
- `tonic` - gRPC client
- `tracing` + `tracing-subscriber` - Logging
- `ross-core` (workspace dependency)

### Daemon Crate
- `clap` (with derive feature) - Command line parsing
- `tokio` (full features) - Async runtime
- `tonic` - gRPC server
- `tracing` + `tracing-subscriber` - Logging
- `ross-core` (workspace dependency)

### Build Dependencies (Core)
- `tonic-build` - Proto compilation

## Proto Definition

Basic service with health check:
```protobuf
syntax = "proto3";
package ross;

service Ross {
    rpc HealthCheck (HealthCheckRequest) returns (HealthCheckResponse);
}

message HealthCheckRequest {}

message HealthCheckResponse {
    bool healthy = 1;
    string version = 2;
}
```

## Dockerfile Specification

- **Base build image:** `rust:alpine`
- **Runtime image:** `alpine:latest`
- **Build type:** Multi-stage for minimal image size
- **Required packages:** `musl-dev`, `protobuf-dev` (build stage)
- **Target:** `x86_64-unknown-linux-musl` for static linking
- **Exposed port:** 50051 (gRPC default)

---

## Development Phases

### Phase 1: Workspace Foundation
**Objective:** Create the cargo workspace structure and root configuration.

**Tasks:**
1. Create root `Cargo.toml` with workspace configuration
2. Create directory structure for all crates
3. Create `proto/ross.proto` with basic service definition

**Files to create:**
- `Cargo.toml` (workspace root)
- `proto/ross.proto`

---

### Phase 2: Core Crate
**Objective:** Set up the core crate with proto compilation and shared types.

**Tasks:**
1. Create `core/Cargo.toml` with dependencies
2. Create `core/build.rs` for tonic proto compilation
3. Create `core/src/lib.rs` that exports generated proto types

**Files to create:**
- `core/Cargo.toml`
- `core/build.rs`
- `core/src/lib.rs`

---

### Phase 3: Daemon Crate
**Objective:** Set up the daemon with clap CLI and gRPC server.

**Tasks:**
1. Create `daemon/Cargo.toml` with dependencies
2. Create `daemon/src/main.rs` with:
   - Clap CLI with `start` subcommand
   - Start command options (host, port)
   - gRPC server setup with tonic
   - Health check service implementation
   - Tracing/logging initialization

**Files to create:**
- `daemon/Cargo.toml`
- `daemon/src/main.rs`

---

### Phase 4: CLI Crate
**Objective:** Set up the CLI with clap foundation for future gRPC client.

**Tasks:**
1. Create `cli/Cargo.toml` with dependencies
2. Create `cli/src/main.rs` with:
   - Clap CLI structure (empty for now, ready for subcommands)
   - Tracing/logging initialization
   - Placeholder for gRPC client connection

**Files to create:**
- `cli/Cargo.toml`
- `cli/src/main.rs`

---

### Phase 5: Dockerfile
**Objective:** Create multi-stage Dockerfile for the daemon.

**Tasks:**
1. Create `Dockerfile` with:
   - Build stage using `rust:alpine`
   - Install build dependencies (musl-dev, protobuf-dev)
   - Build release binary with musl target
   - Runtime stage using `alpine:latest`
   - Copy binary and set entrypoint
   - Expose gRPC port 50051

**Files to create:**
- `Dockerfile`
- `.dockerignore` (optional, for faster builds)

---

## Expected Final Structure

```
ross/
├── Cargo.toml
├── Cargo.lock (generated)
├── Dockerfile
├── .dockerignore
├── DEVELOPMENT_PLAN.md
├── proto/
│   └── ross.proto
├── core/
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/
│       └── lib.rs
├── cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── daemon/
    ├── Cargo.toml
    └── src/
        └── main.rs
```

## Verification Commands

After all phases complete:
```bash
# Build entire workspace
cargo build

# Run daemon
cargo run -p ross-daemon -- start --port 50051

# Run CLI (placeholder)
cargo run -p ross-cli

# Build Docker image
docker build -t ross-daemon .

# Run Docker container
docker run -p 50051:50051 ross-daemon
```
