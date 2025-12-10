# Ross Architecture Documentation

Welcome to the Ross container runtime architecture documentation. This directory contains detailed technical documentation about the design and implementation of Ross.

## Documentation Index

### 📘 [Overview](./overview.md)
High-level architecture and component relationships. Start here to understand the system structure.

**Topics covered:**
- Architecture philosophy
- System components diagram
- Component responsibilities
- Data flow overview
- Storage layout
- Technology stack

### 🧩 [Components](./components.md)
Detailed descriptions of each crate/component in the Ross workspace.

**Components documented:**
- ross-core (gRPC types)
- ross-cli (command-line interface)
- ross-daemon (gRPC server)
- ross-container (container lifecycle)
- ross-image (image management)
- ross-store (content-addressable storage)
- ross-snapshotter (overlay filesystem)
- ross-shim (runc interface)
- ross-remote (registry client)
- ross-mount (overlay mounting)

### 📥 [Image Pulling](./image-pulling.md)
Complete flow of pulling container images from registries.

**Topics covered:**
- Reference resolution
- Registry authentication
- Manifest resolution (single and multi-platform)
- Config and layer downloads
- Concurrent download management
- Layer extraction
- Progress reporting
- Error handling

### 🔄 [Container Lifecycle](./container-lifecycle.md)
How containers are created, started, stopped, and removed.

**Topics covered:**
- Container states and transitions
- Create flow (snapshot preparation, OCI spec generation)
- Start flow (runc execution)
- Interactive sessions (PTY handling)
- Stop and signal handling
- Remove and cleanup
- Logs and inspection

### 💾 [Storage](./storage.md)
Content-addressable storage architecture.

**Topics covered:**
- Storage philosophy (deduplication, integrity, immutability)
- Digest format and structure
- Directory structure
- Blobs (layers, configs)
- Manifests (image descriptors)
- Tags (human-readable references)
- Content deduplication
- Garbage collection
- Integrity verification

### 📸 [Snapshotter](./snapshotter.md)
Overlay filesystem and layer management.

**Topics covered:**
- OverlayFS primer
- Snapshot types (View, Active, Committed)
- Snapshot lifecycle
- Layer extraction from compressed tarballs
- Whiteout handling
- Parent chains
- Mount specifications
- Performance characteristics

### 🌐 [Networking](./networking.md)
gRPC communication and protocols.

**Topics covered:**
- Protocol Buffer definitions
- Service definitions (Container, Image, Snapshotter)
- Communication patterns (unary, streaming, bidirectional)
- Type conversion layer
- Error handling and status codes
- Authentication and security
- Performance considerations

### 🔀 [Data Flow](./data-flow.md)
Detailed data flow diagrams for key operations.

**Topics covered:**
- Image pull data flow (end-to-end)
- Container run data flow (interactive mode)
- Container logs data flow
- Storage read/write paths
- Memory flow patterns
- Disk I/O patterns
- Network traffic patterns
- Concurrency patterns
- Error propagation

## Quick Reference

### Key Concepts

**Content-Addressable Storage**
```
All artifacts stored by their SHA-256 digest
Example: blobs/sha256/d4ff818577bc...
```

**Overlay Filesystem**
```
Layered filesystem with copy-on-write
lowerdir (read-only layers) + upperdir (writable) = merged view
```

**Snapshot Chain**
```
container → layer3 → layer2 → layer1
Each snapshot can have a parent forming a chain
```

**gRPC Services**
```
ContainerService - Container operations
ImageService - Image operations  
SnapshotterService - Snapshot operations
```

### Architecture Principles

1. **Separation of Concerns**: Core logic decoupled from transport (gRPC)
2. **Content Addressing**: Artifacts referenced by cryptographic digest
3. **Layer Sharing**: Multiple images share common layers
4. **Immutability**: Content cannot change without changing digest
5. **Streaming**: Long operations stream progress/output

### Common Workflows

**Pull an Image**
```
CLI → Daemon → ImageService → RegistryClient → Registry
                    ↓
              Store (blobs, manifests, tags)
                    ↓
              Snapshotter (extract layers)
```

**Run a Container**
```
CLI → Daemon → ContainerService → resolve image
                    ↓
              Snapshotter (prepare snapshot)
                    ↓
              Shim (generate OCI spec, mount overlay)
                    ↓
              runc (execute container)
```

## Directory Structure

The Ross data directory (`/var/lib/ross/`) contains:

```
/var/lib/ross/
├── blobs/          # Content-addressed blobs
│   └── sha256/     # Organized by digest algorithm
├── manifests/      # Image manifests
│   └── sha256/
├── tags/           # Tag → Manifest mappings
│   └── {repo}/
│       └── {tag}
├── snapshots/      # Overlay snapshots
│   └── {key}/
│       ├── fs/     # Filesystem content
│       ├── work/   # Overlay work directory
│       └── metadata.json
└── containers/     # Container bundles
    └── {id}/
        ├── bundle/
        │   ├── config.json  # OCI spec
        │   └── rootfs/      # Mounted overlay
        └── metadata.json
```

## Technology Stack

- **Language**: Rust (2021 edition)
- **RPC**: gRPC via tonic
- **Serialization**: Protocol Buffers, JSON
- **Container Runtime**: runc (OCI-compliant)
- **Filesystem**: OverlayFS
- **Registry Protocol**: OCI Distribution Spec
- **Hashing**: SHA-256
- **Compression**: gzip

## Contributing to Documentation

When adding or updating architecture documentation:

1. **Keep files focused**: Each file should cover one major topic
2. **Use diagrams**: ASCII art diagrams are encouraged
3. **Include examples**: Show concrete code/data examples
4. **Cross-reference**: Link to related documentation
5. **Update this README**: Add new documents to the index

## Additional Resources

- **Main README**: [`../../README.md`](../../README.md) - Project overview and getting started
- **Development Guide**: [`../../DEVELOPMENT.md`](../../DEVELOPMENT.md) - Setup and development workflow
- **Protocol Buffers**: [`../../proto/`](../../proto/) - gRPC service definitions
- **Source Code**: [`../../src/`](../../src/) - Implementation

## Questions?

If you have questions about the architecture that aren't covered in these documents:

1. Check the source code comments
2. Look at the protocol buffer definitions
3. Review the tests for usage examples
4. Open an issue on GitHub

## Document Status

- ✅ Overview - Complete
- ✅ Components - Complete
- ✅ Image Pulling - Complete
- ✅ Container Lifecycle - Complete
- ✅ Storage - Complete
- ✅ Snapshotter - Complete
- ✅ Networking - Complete
- ✅ Data Flow - Complete

Last updated: 2024-12-10
