# Ross Components

This document provides detailed information about each component in the Ross container runtime.

## ross-core

**Location**: `core/`  
**Type**: Library  
**Dependencies**: `tonic`, `prost`

### Purpose
Provides generated gRPC service definitions and message types from Protocol Buffer specifications.

### Key Exports
- `ContainerServiceServer` - gRPC service trait for container operations
- `ImageServiceServer` - gRPC service trait for image operations
- `SnapshotterServiceServer` - gRPC service trait for snapshot operations
- All request/response message types

### Build Process
Uses `tonic-build` in `build.rs` to compile `.proto` files from `proto/` directory at compile time.

---

## ross-cli

**Location**: `cli/`  
**Type**: Binary  
**Dependencies**: `ross-core`, `clap`, `tokio`

### Purpose
Command-line interface for interacting with the Ross daemon.

### Key Features
- Container commands: `create`, `start`, `stop`, `rm`, `ps`, `logs`, `exec`
- Image commands: `pull`, `push`, `images`, `rmi`, `tag`
- Interactive run: `run -it` for terminal sessions
- Health check: `health` command

### Implementation Details
- Connects to daemon via gRPC (default: `http://127.0.0.1:50051`)
- Uses generated gRPC client stubs from ross-core
- Formats output for human readability
- Handles streaming responses (pull progress, logs, etc.)

---

## ross-daemon

**Location**: `daemon/`  
**Type**: Binary  
**Dependencies**: `ross-core`, `ross-container`, `ross-image`, `ross-store`, `ross-snapshotter`

### Purpose
gRPC server that exposes container and image services over the network.

### Architecture
Thin adapter layer with three main service implementations:

#### Container Service Adapter (`services/container.rs`)
```rust
pub struct ContainerServiceImpl {
    service: Arc<ross_container::ContainerService>,
}
```
- Converts gRPC types → domain types → gRPC types
- Delegates all logic to `ross_container::ContainerService`
- Handles streaming responses (logs, attach, wait)

#### Image Service Adapter (`services/image.rs`)
```rust
pub struct ImageServiceImpl {
    service: Arc<ross_image::ImageService>,
}
```
- Converts gRPC types → domain types → gRPC types
- Delegates all logic to `ross_image::ImageService`
- Streams pull/push progress

#### Snapshotter Service Adapter (`services/snapshotter.rs`)
```rust
pub struct SnapshotterServiceImpl {
    snapshotter: Arc<ross_snapshotter::OverlaySnapshotter>,
}
```
- Exposes snapshotter operations via gRPC
- Direct thin wrapper over snapshotter

### Type Conversion Pattern
```rust
// gRPC → Domain
fn container_config_from_grpc(c: ross_core::ContainerConfig) -> ross_container::ContainerConfig {
    ross_container::ContainerConfig {
        image: c.image,
        hostname: c.hostname,
        cmd: c.cmd,
        // ...
    }
}

// Domain → gRPC
fn container_to_grpc(c: ross_container::Container) -> ross_core::Container {
    ross_core::Container {
        id: c.id,
        names: c.names,
        image: c.image,
        // ...
    }
}
```

---

## ross-container

**Location**: `container/`  
**Type**: Library  
**Dependencies**: `ross-shim`, `ross-snapshotter`, `ross-store`, `tokio`

### Purpose
Core container lifecycle management, completely decoupled from gRPC.

### Key Types
```rust
pub struct ContainerService {
    shim: Arc<RuncShim>,
    snapshotter: Arc<OverlaySnapshotter>,
    store: Arc<FileSystemStore>,
}

pub struct Container {
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub state: String,
    pub created: Option<Timestamp>,
    // ...
}

pub struct ContainerConfig {
    pub image: String,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: String,
    // ...
}
```

### Key Operations

#### Create Container
1. Parse image reference and resolve tag
2. Fetch image manifest and config from store
3. Find top layer digest
4. Prepare snapshot with overlay mount
5. Merge image config with user config
6. Create OCI spec via shim
7. Return container ID

#### Start Container
1. Verify container state (must be Created)
2. Call shim to start container via runc
3. Update container state to Running

#### Run Interactive
1. Create container
2. Start bidirectional streaming
3. Set up PTY for terminal I/O
4. Forward stdin/stdout between client and container
5. Handle terminal resize events

### Image Config Merging
User-provided config takes precedence over image defaults:
```rust
let entrypoint = if params.config.entrypoint.is_empty() {
    image_config.entrypoint  // Use image default
} else {
    params.config.entrypoint.clone()  // User override
};
```

---

## ross-image

**Location**: `image/`  
**Type**: Library  
**Dependencies**: `ross-remote`, `ross-store`, `ross-snapshotter`, `tokio`

### Purpose
Image management operations, completely decoupled from gRPC.

### Key Types
```rust
pub struct ImageService {
    store: Arc<FileSystemStore>,
    snapshotter: Arc<OverlaySnapshotter>,
    max_concurrent_downloads: usize,
}

pub struct Image {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub architecture: String,
    pub os: String,
    pub size: i64,
    pub labels: HashMap<String, String>,
    pub root_fs: Option<RootFs>,
}

pub struct PullProgress {
    pub id: String,
    pub status: String,
    pub progress: String,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub error: Option<String>,
}
```

### Pull Image Flow
1. **Parse reference**: Extract registry, repository, tag
2. **Resolve manifest**: Contact registry, handle auth
3. **Pull config**: Download and store image config blob
4. **Download layers**: Parallel downloads with semaphore
5. **Store artifacts**: Save blobs and manifests by digest
6. **Tag reference**: Create tag → manifest mapping
7. **Extract layers**: Uncompress tarballs into snapshots
8. **Build chain**: Create parent-child snapshot relationships

### Concurrent Downloads
Uses semaphore to limit parallel layer downloads:
```rust
let semaphore = Arc::new(Semaphore::new(max_concurrent));
for layer in manifest.layers {
    tokio::spawn(download_layer(..., semaphore.clone()));
}
```

---

## ross-store

**Location**: `store/`  
**Type**: Library  
**Dependencies**: `tokio`, `sha2`, `serde`

### Purpose
Content-addressable storage for all image artifacts.

### Key Types
```rust
pub struct FileSystemStore {
    root: PathBuf,
}

pub struct Digest {
    pub algorithm: String,  // "sha256"
    pub hash: String,       // hex-encoded
}
```

### Storage Structure
```
root/
├── blobs/
│   └── sha256/
│       ├── {hash}          # Blob content
│       └── {hash}.meta     # Metadata JSON
├── manifests/
│   └── sha256/
│       ├── {hash}          # Manifest content
│       └── {hash}.meta     # Metadata JSON
├── indexes/
│   └── sha256/
│       └── {hash}          # Image index
└── tags/
    └── {repository}/
        └── {tag}           # Tag metadata JSON
```

### Metadata Files

**Blob Metadata**:
```json
{
  "media_type": "application/vnd.oci.image.layer.v1.tar+gzip",
  "size": 1048576,
  "created_at": 1699564800,
  "accessed_at": 1699564800
}
```

**Tag Metadata**:
```json
{
  "digest_algorithm": "sha256",
  "digest_hash": "abc123...",
  "updated_at": 1699564800
}
```

### Key Operations

#### Put Blob
```rust
async fn put_blob(
    &self,
    media_type: &str,
    data: &[u8],
    expected_digest: Option<&Digest>,
) -> Result<(Digest, i64), StoreError>
```
1. Hash data with SHA-256
2. Verify against expected digest if provided
3. Write blob to `blobs/{algorithm}/{hash}`
4. Write metadata to `blobs/{algorithm}/{hash}.meta`
5. Return computed digest and size

#### Get Blob
Supports range reads for efficient partial downloads:
```rust
async fn get_blob(
    &self,
    digest: &Digest,
    offset: i64,
    length: i64,  // -1 means read to end
) -> Result<Vec<u8>, StoreError>
```

#### Garbage Collection
Identifies unreferenced artifacts:
1. Scan all tags to find referenced digests
2. List all manifests
3. Delete untagged manifests if requested
4. Return space reclaimed

---

## ross-snapshotter

**Location**: `snapshotter/`  
**Type**: Library  
**Dependencies**: `ross-store`, `flate2`, `tar`, `tokio`

### Purpose
Manages filesystem snapshots using overlay filesystem for efficient layer composition.

### Key Types
```rust
pub struct OverlaySnapshotter {
    root: PathBuf,
    store: Arc<FileSystemStore>,
    snapshots: RwLock<HashMap<String, SnapshotInfo>>,
}

pub struct SnapshotInfo {
    pub key: String,
    pub parent: Option<String>,
    pub kind: SnapshotKind,
    pub created_at: i64,
    pub updated_at: i64,
    pub labels: HashMap<String, String>,
}

pub enum SnapshotKind {
    View,       // Read-only view
    Active,     // Writable snapshot
    Committed,  // Immutable committed snapshot
}

pub struct Mount {
    pub mount_type: String,    // "overlay" or "bind"
    pub source: String,
    pub target: String,
    pub options: Vec<String>,
}
```

### Snapshot Structure
```
snapshots/
├── {snapshot-key}/
│   ├── fs/              # Snapshot filesystem content
│   ├── work/            # OverlayFS work directory
│   └── metadata.json    # Snapshot metadata
```

### Key Operations

#### Prepare
Creates an active (writable) snapshot:
```rust
async fn prepare(
    &self,
    key: &str,
    parent: Option<&str>,
    labels: HashMap<String, String>,
) -> Result<Vec<Mount>, SnapshotterError>
```
1. Validate parent exists and is committed
2. Create snapshot directory structure
3. Save metadata
4. Build overlay mount specification
5. Return mounts for consumer

#### View
Creates a read-only view snapshot (no upper/work dirs):
```rust
async fn view(
    &self,
    key: &str,
    parent: Option<&str>,
    labels: HashMap<String, String>,
) -> Result<Vec<Mount>, SnapshotterError>
```

#### Commit
Converts active snapshot to committed:
```rust
async fn commit(
    &self,
    key: &str,
    active_key: &str,
    labels: HashMap<String, String>,
) -> Result<(), SnapshotterError>
```
1. Verify active snapshot exists
2. Rename snapshot directory
3. Update metadata with new key
4. Change kind to Committed

#### Extract Layer
Extracts compressed tar layer into snapshot:
```rust
async fn extract_layer(
    &self,
    digest: &str,
    parent_key: Option<&str>,
    key: &str,
    labels: HashMap<String, String>,
) -> Result<(String, i64), SnapshotterError>
```
1. Fetch compressed blob from store
2. Create temporary active snapshot
3. Extract tar.gz into snapshot filesystem
4. Handle whiteout files (`.wh.` prefix)
5. Commit snapshot with layer digest label
6. Return snapshot key and extracted size

### Overlay Mount Construction

**Single Layer** (no parent):
```
mount -t bind {fs_dir} {target}
```

**Layered** (with parent chain):
```
mount -t overlay overlay -o \
  lowerdir=layer3:layer2:layer1,\
  upperdir={fs_dir},\
  workdir={work_dir} \
  {target}
```

The parent chain is traversed from child to root, building a colon-separated list of lower directories.

### Whiteout Handling
OCI layers use `.wh.{filename}` to indicate deleted files:
```rust
if name.starts_with(".wh.") {
    let original_name = name.strip_prefix(".wh.").unwrap();
    remove_file_or_dir(original_name);
    continue;  // Don't extract the whiteout marker
}
```

---

## ross-shim

**Location**: `shim/`  
**Type**: Library  
**Dependencies**: `runc`, `oci-spec`, `ross-mount`, `tokio`

### Purpose
Interfaces with runc for OCI-compliant container execution.

### Key Types
```rust
pub struct RuncShim {
    runc: Runc,
    data_dir: PathBuf,
    containers: Arc<RwLock<HashMap<String, ContainerMetadata>>>,
}

pub struct ContainerInfo {
    pub id: String,
    pub name: Option<String>,
    pub image: String,
    pub state: ContainerState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub bundle_path: String,
    pub rootfs_path: String,
}

pub enum ContainerState {
    Created,
    Running,
    Paused,
    Stopped,
}
```

### Container Bundle
Each container gets a bundle directory:
```
containers/{container-id}/
├── bundle/
│   ├── config.json       # OCI runtime spec
│   ├── rootfs/           # Mounted overlay filesystem
│   ├── container.pid     # Container process PID
│   ├── stdout.log        # Stdout capture
│   └── stderr.log        # Stderr capture
└── metadata.json         # Ross container metadata
```

### OCI Spec Generation
Converts container config to OCI runtime specification:
```rust
fn generate_spec(&self, opts: &CreateContainerOpts, rootfs: &Path) 
    -> Result<Spec, ShimError>
```

**Process Configuration**:
- Args: entrypoint + cmd
- User: UID/GID from user string
- Working directory
- Environment variables
- Terminal flag

**Root Configuration**:
- Path: mounted rootfs
- Readonly: from host config

**Mounts**:
- Standard mounts: `/proc`, `/dev`, `/dev/pts`, `/dev/shm`, `/sys`
- User bind mounts from host config

**Namespaces**:
- PID, IPC, UTS, Mount (always)
- Network (unless host networking)

**Example OCI Spec**:
```json
{
  "ociVersion": "1.0.2",
  "process": {
    "terminal": true,
    "user": {"uid": 0, "gid": 0},
    "args": ["/bin/sh"],
    "env": ["PATH=/usr/local/bin:/usr/bin:/bin"],
    "cwd": "/"
  },
  "root": {
    "path": "rootfs",
    "readonly": false
  },
  "mounts": [
    {"destination": "/proc", "type": "proc", "source": "proc"},
    {"destination": "/dev", "type": "tmpfs", "source": "tmpfs"},
    ...
  ],
  "linux": {
    "namespaces": [
      {"type": "pid"},
      {"type": "network"},
      {"type": "mount"},
      ...
    ]
  }
}
```

### Runc Integration

**Create**: Prepares bundle, mounts rootfs, generates spec
```bash
# Internally called
runc create --bundle /bundle/path container-id
```

**Start**: Executes container process
```bash
runc start container-id
```

**Stop**: Sends SIGTERM, waits, then SIGKILL
```bash
runc kill container-id 15
# wait timeout
runc kill container-id 9
```

### Interactive Sessions
For `docker run -it` equivalent:

1. Create Unix socket for console
2. Start runc with `--console-socket`
3. Receive PTY master fd via socket
4. Set PTY to raw mode
5. Bidirectional copy between client and PTY

```rust
pub async fn run_interactive(
    &self,
    id: String,
    input_rx: mpsc::Receiver<InputEvent>,
    output_tx: mpsc::Sender<OutputEvent>,
) -> Result<(), ShimError>
```

Handles:
- Stdin forwarding to PTY
- PTY output streaming to client
- Terminal resize events
- Exit code propagation

---

## ross-remote

**Location**: `remote/`  
**Type**: Library  
**Dependencies**: `reqwest`, `serde`, `tokio`

### Purpose
Registry client for pulling/pushing images from OCI-compliant registries.

### Key Types
```rust
pub struct RegistryClient {
    client: Client,
    tokens: Arc<RwLock<HashMap<String, String>>>,
}

pub struct ImageReference {
    pub registry: String,    // "docker.io"
    pub repository: String,  // "library/nginx"
    pub tag: Option<String>, // "latest"
    pub digest: Option<String>,
}

pub enum Manifest {
    V2(ManifestV2),
    List(ManifestList),
}

pub struct ManifestV2 {
    pub schema_version: i32,
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
}
```

### Registry API Flow

#### Authentication
1. Make unauthenticated request
2. Receive 401 with `WWW-Authenticate` header
3. Parse realm, service, scope
4. Request token from auth endpoint
5. Cache token for repository
6. Retry request with `Authorization: Bearer {token}`

```rust
async fn authenticate(
    &self,
    reference: &ImageReference,
    www_auth: &str,
) -> Result<String, RegistryError>
```

#### Get Manifest
```rust
async fn get_manifest(
    &self,
    reference: &ImageReference,
) -> Result<(Manifest, String, String), RegistryError>
```
1. Construct URL: `{registry}/v2/{repo}/manifests/{tag_or_digest}`
2. Set Accept headers for all manifest types
3. Authenticate if needed
4. Parse response as manifest or manifest list
5. Return manifest, media type, and digest

#### Platform Resolution
For multi-platform images:
```rust
async fn get_manifest_for_platform(
    &self,
    reference: &ImageReference,
    os: &str,
    arch: &str,
) -> Result<(ManifestV2, String, String), RegistryError>
```
1. Fetch manifest (might be a list)
2. If manifest list, find matching platform entry
3. Fetch platform-specific manifest by digest
4. Return resolved manifest

#### Get Blob
```rust
async fn get_blob_bytes(
    &self,
    reference: &ImageReference,
    digest: &str,
) -> Result<Vec<u8>, RegistryError>
```
Downloads blob (layer or config) by digest:
- URL: `{registry}/v2/{repo}/blobs/{digest}`
- Returns raw bytes

### Media Types
- `application/vnd.docker.distribution.manifest.v2+json`
- `application/vnd.docker.distribution.manifest.list.v2+json`
- `application/vnd.oci.image.manifest.v1+json`
- `application/vnd.oci.image.index.v1+json`

---

## ross-mount

**Location**: `mount/`  
**Type**: Library  
**Dependencies**: `nix`

### Purpose
Low-level overlay filesystem mounting operations.

### Key Types
```rust
pub struct MountSpec {
    pub fstype: String,
    pub source: String,
    pub options: Vec<String>,
}
```

### Operations

#### Mount Overlay
```rust
pub fn mount_overlay(spec: &MountSpec, target: &Path) -> Result<(), MountError>
```
Creates overlay mount with proper options:
```rust
mount(
    Some(spec.source.as_str()),
    target,
    Some(spec.fstype.as_str()),
    MsFlags::MS_NOATIME,
    Some(spec.options.join(",").as_str()),
)
```

#### Unmount
```rust
pub fn unmount(target: &Path) -> Result<(), MountError>
```
Unmounts filesystem:
```rust
umount2(target, MntFlags::MNT_DETACH)
```

### OverlayFS Details
Overlay mounts require specific options:
```
lowerdir=layer3:layer2:layer1  # Read-only layers (right to left)
upperdir=/path/to/upper         # Writable layer
workdir=/path/to/work           # Overlay work directory
```

Work directory must be:
- On same filesystem as upperdir
- Empty directory
- Used by kernel for atomic operations
