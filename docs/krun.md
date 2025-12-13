# Running Ross with libkrun

This document explains how to use Ross with libkrun to run containers in lightweight virtual machines on macOS.

## Overview

[libkrun](https://github.com/containers/libkrun) is a library that allows running workloads in isolated micro-VMs using Apple's Virtualization.framework on macOS (and KVM on Linux). When Ross is built with libkrun support, containers run inside lightweight VMs instead of using traditional Linux namespaces, providing stronger isolation.

## Prerequisites

### macOS

1. **macOS 11 (Big Sur) or later** - Required for Virtualization.framework support

2. **Install libkrun** via Homebrew:
   ```bash
   brew install libkrun
   ```

3. **Rust toolchain** - Install via [rustup](https://rustup.rs/)

## Building Ross with libkrun Support

Build the shim crate with the `libkrun` feature enabled:

```bash
cargo build --release -p ross-shim --features libkrun
```

To build the entire project with libkrun support:

```bash
cargo build --release --features ross-shim/libkrun
```

## Architecture

When using libkrun, Ross creates a lightweight VM for each container:

```
┌─────────────────────────────────────────┐
│              Host (macOS)               │
│                                         │
│  ┌─────────────────────────────────┐    │
│  │         Ross Daemon             │    │
│  │                                 │    │
│  │  ┌───────────────────────────┐  │    │
│  │  │       KrunShim            │  │    │
│  │  │                           │  │    │
│  │  │  ┌─────────┐ ┌─────────┐  │  │    │
│  │  │  │  VM 1   │ │  VM 2   │  │  │    │
│  │  │  │(cont-a) │ │(cont-b) │  │  │    │
│  │  │  └─────────┘ └─────────┘  │  │    │
│  │  └───────────────────────────┘  │    │
│  └─────────────────────────────────┘    │
└─────────────────────────────────────────┘
```

Each container runs in its own micro-VM with:
- Dedicated vCPUs (default: 2)
- Dedicated RAM (default: 512 MB)
- Isolated root filesystem
- Console I/O via virtio-console

### Root Filesystem Preparation

Unlike traditional container runtimes that use overlayfs to layer filesystems, libkrun requires a single directory containing the complete filesystem. Ross handles this automatically by:

1. **For pulled images**: Copying all overlay layers (lowerdir + upperdir) into a single merged directory
2. **Whiteout handling**: Properly processing OCI whiteout files (`.wh.*`) to delete files from lower layers
3. **Essential directories**: Ensuring required directories (`/dev`, `/proc`, `/sys`, `/tmp`, etc.) exist

The rootfs preparation happens automatically during container creation. You don't need to do anything special - just pull an image and run it.

## Usage

### Starting the Daemon

The daemon needs to be configured to use the libkrun shim. This is typically done through configuration or command-line flags (implementation-dependent).

```bash
cargo run --release -p ross-daemon -- start
```

### Running Containers

Once the daemon is running with libkrun support, use the CLI as normal:

```bash
# Pull an image
ross-cli image pull alpine:latest

# Create and run a container
ross-cli container run alpine:latest /bin/sh -c "echo Hello from VM"

# Run interactively
ross-cli container run -it alpine:latest /bin/sh
```

### Volumes (bind mounts)

When running with libkrun, Ross exposes host directories to the guest using **virtio-fs**
and mounts them inside the VM before executing the container command.

Use `--volume/-v` with the format:

- `HOST_PATH:GUEST_PATH`
- `HOST_PATH:GUEST_PATH:ro` (read-only)

Examples:

```bash
# Mount a host directory at /data inside the container/VM
ross-cli container run -it --volume /Users/rumpl/data:/data alpine:latest /bin/sh

# Read-only mount
ross-cli container run --volume /Users/rumpl/config:/etc/config:ro alpine:latest /bin/cat /etc/config/example.conf
```

### Container Lifecycle

```bash
# Create a container
ross-cli container create --name mycontainer alpine:latest

# Start the container
ross-cli container start mycontainer

# View logs
ross-cli container logs mycontainer

# Stop the container
ross-cli container stop mycontainer

# Remove the container
ross-cli container delete mycontainer
```

## Configuration

### VM Resources

The default VM configuration is:
- **vCPUs**: 2
- **RAM**: 512 MB

These can be adjusted in the `KrunShim` implementation or through container configuration options (when supported).

## Limitations

1. **macOS only** - The libkrun feature currently only works on macOS with Apple Silicon or Intel processors that support Virtualization.framework.

2. **No network namespaces** - Network configuration differs from traditional container runtimes. libkrun uses virtio-net for networking.

3. **Resource overhead** - Each container runs in a separate VM, which has more overhead than namespace-based isolation.

4. **Disk space** - Since libkrun requires a flat rootfs (no overlayfs), each container gets a full copy of the image layers, using more disk space.

5. **Image format** - Images must be compatible with the VM's init system. Standard OCI images work, but the entrypoint must be a valid executable for the VM environment.

## Troubleshooting

### "libkrun support not available"

This error means Ross was not built with libkrun support. Rebuild with:
```bash
cargo build --release -p ross-shim --features libkrun
```

### "Failed to create libkrun context"

Ensure libkrun is properly installed:
```bash
brew install libkrun
```

Also verify that your macOS version supports Virtualization.framework (macOS 11+).

### VM fails to start

Check that:
1. The root filesystem path exists and is accessible
2. The entrypoint executable exists in the root filesystem
3. You have sufficient permissions to use Virtualization.framework

### Console not working

If interactive mode doesn't work properly, ensure:
1. The terminal is a proper TTY
2. The container's shell is available in the root filesystem

### Large container sizes

Since libkrun requires copying all layers into a flat directory, container rootfs directories can be large. Consider:
- Using smaller base images (alpine vs ubuntu)
- Cleaning up containers promptly with `ross-cli container delete`

## Development

### Running Tests

```bash
cargo test -p ross-shim --features libkrun
```

### Debugging

Enable debug logging:
```bash
RUST_LOG=debug cargo run -p ross-daemon -- start
```

libkrun also supports its own logging which can be enabled by setting the log level in the `KrunContext`.

## Implementation Details

### Key Files

- `shim/src/libkrun_shim.rs` - Main KrunShim implementation
- `shim/src/rootfs.rs` - Rootfs preparation utilities
- `shim/src/error.rs` - Error types

### Rootfs Module (`shim/src/rootfs.rs`)

Provides utilities for preparing root filesystems:

- `extract_layer()` - Extracts a gzipped tar layer with whiteout handling
- `prepare_rootfs()` - Creates a merged rootfs from multiple layers
- `create_minimal_rootfs()` - Creates a minimal filesystem for testing
- `ensure_essential_dirs()` - Ensures required Linux directories exist

## Further Reading

- [libkrun GitHub](https://github.com/containers/libkrun)
- [krun-sys crate](https://crates.io/crates/krun-sys)
- [Apple Virtualization.framework](https://developer.apple.com/documentation/virtualization)
- [OCI Image Spec - Whiteouts](https://github.com/opencontainers/image-spec/blob/main/layer.md#whiteouts)
