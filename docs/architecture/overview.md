# Ross Architecture Overview

Ross is a container runtime written in Rust, designed with a modular architecture that separates concerns across multiple crates. This document provides a high-level overview of the system.

## Architecture Philosophy

Ross follows these key principles:

1. **Separation of Concerns**: Core business logic is decoupled from transport mechanisms (gRPC)
2. **Content-Addressable Storage**: All artifacts (blobs, manifests) are stored by their digest
3. **Layered Filesystem**: Uses overlay filesystem for efficient layer composition
4. **OCI Compliance**: Follows OCI specifications for images and runtime

## System Components

```
┌─────────────────────────────────────────────────────────────────┐
│                         ross-cli                                 │
│                    (Command Line Interface)                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ gRPC
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        ross-daemon                               │
│                   (gRPC Server & Adapter)                        │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────┐ │
│  │ Container Service│  │  Image Service   │  │ Snapshotter  │ │
│  │    Adapter       │  │    Adapter       │  │   Adapter    │ │
│  └──────────────────┘  └──────────────────┘  └──────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Core Services Layer                         │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────┐ │
│  │ ross-container   │  │   ross-image     │  │ross-snapshotter│
│  │                  │  │                  │  │               │ │
│  │ Container Mgmt   │  │  Image Mgmt      │  │ Layer Mgmt    │ │
│  └──────────────────┘  └──────────────────┘  └──────────────┘ │
│           │                     │                     │         │
│           └─────────────────────┴─────────────────────┘         │
│                                 │                               │
└─────────────────────────────────────────────────────────────────┘
                                  │
                ┌─────────────────┴─────────────────┐
                ▼                                   ▼
┌──────────────────────────────┐    ┌────────────────────────────┐
│        ross-shim             │    │      ross-store            │
│                              │    │                            │
│  ┌────────────────────────┐ │    │  Content-Addressable Store │
│  │     RuncShim           │ │    │  - Blobs                   │
│  │  - OCI spec generation │ │    │  - Manifests               │
│  │  - runc orchestration  │ │    │  - Tags                    │
│  │  - Process management  │ │    │  - Indexes                 │
│  └────────────────────────┘ │    │                            │
└──────────────────────────────┘    └────────────────────────────┘
                │                                   ▲
                │                                   │
                ▼                                   │
┌──────────────────────────────┐    ┌────────────────────────────┐
│       ross-mount             │    │     ross-remote            │
│                              │    │                            │
│  Overlay FS mounting         │    │  Registry Client           │
│  - OverlayFS setup           │    │  - Image pulling           │
│  - Mount management          │    │  - Authentication          │
│                              │    │  - Manifest resolution     │
└──────────────────────────────┘    └────────────────────────────┘
                                                   │
                                                   ▼
                                    ┌────────────────────────────┐
                                    │   Container Registries     │
                                    │   (Docker Hub, etc.)       │
                                    └────────────────────────────┘
```

## Component Responsibilities

### ross-cli
User-facing command-line interface. Communicates with ross-daemon via gRPC.

### ross-daemon
- gRPC server exposing container and image services
- Thin adapter layer converting between gRPC types and domain types
- No business logic - delegates to core services

### ross-container
- Container lifecycle management (create, start, stop, remove)
- Container state tracking
- Delegates to ross-shim for actual container execution

### ross-image
- Image management (pull, push, list, tag)
- Orchestrates image pulling from registries
- Coordinates with store and snapshotter for layer extraction

### ross-store
- Content-addressable storage for all artifacts
- Stores blobs (layers, configs) by digest
- Stores manifests and maintains tag references
- Provides garbage collection

### ross-snapshotter
- Manages filesystem snapshots using overlay filesystem
- Creates layered filesystem from image layers
- Maintains snapshot metadata and parent chains
- Handles layer extraction from compressed tarballs

### ross-shim
- Interfaces with runc for container execution
- Generates OCI runtime specifications
- Manages container processes and lifecycle
- Handles interactive sessions with PTY support

### ross-remote
- Registry client for OCI-compliant registries
- Handles authentication and token management
- Downloads manifests and blobs
- Resolves multi-platform manifests

### ross-mount
- Low-level overlay filesystem mounting
- Creates and tears down overlay mounts
- Handles mount options and filesystem setup

### ross-core
- Generated gRPC types from protocol buffer definitions
- Shared between CLI and daemon for communication

## Data Flow Overview

### Image Pull Flow
```
CLI → Daemon → ImageService → RegistryClient → Registry
                    ↓
              Store (save blobs/manifests)
                    ↓
              Snapshotter (extract layers)
```

### Container Run Flow
```
CLI → Daemon → ContainerService → Snapshotter (prepare mount)
                    ↓
               Shim (generate OCI spec)
                    ↓
               runc (start container)
```

## Storage Layout

```
/var/lib/ross/
├── blobs/           # Content-addressable blobs
│   └── sha256/
│       ├── abc123...
│       └── def456...
├── manifests/       # Image manifests
│   └── sha256/
│       └── manifest-hash...
├── tags/            # Tag → Digest mappings
│   └── library/
│       └── nginx/
│           └── latest
├── snapshots/       # Overlay snapshots
│   ├── layer-1/
│   │   ├── fs/      # Layer content
│   │   ├── work/    # Overlay work dir
│   │   └── metadata.json
│   └── container-uuid/
│       ├── fs/
│       ├── work/
│       └── metadata.json
└── containers/      # Container state
    └── container-id/
        ├── bundle/
        │   ├── config.json  # OCI spec
        │   └── rootfs/      # Mounted overlay
        └── metadata.json
```

## Technology Stack

- **Language**: Rust
- **RPC Framework**: gRPC (tonic)
- **Container Runtime**: runc (OCI-compliant)
- **Filesystem**: OverlayFS
- **Hashing**: SHA-256
- **Compression**: gzip (for layer tarballs)
- **Serialization**: JSON (metadata), Protocol Buffers (gRPC)

## Next Steps

For detailed information about specific subsystems:
- [Components](./components.md) - Detailed component descriptions
- [Image Pulling](./image-pulling.md) - Image pull flow and diagrams
- [Container Lifecycle](./container-lifecycle.md) - Container creation and execution
- [Storage](./storage.md) - Content-addressable storage architecture
- [Snapshotter](./snapshotter.md) - Overlay snapshotter and layer management
- [Networking](./networking.md) - gRPC communication and protocols
- [Data Flow](./data-flow.md) - Detailed data flow diagrams
