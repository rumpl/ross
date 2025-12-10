# Data Flow Architecture

This document describes how data flows through the Ross system for key operations.

## Image Pull Data Flow

```
┌──────────┐
│   CLI    │
└──────────┘
     │ 1. ross-cli image pull nginx:latest
     ▼
┌──────────────────────────────────────────────────────────┐
│                    gRPC Layer                             │
│  PullImageRequest { image_name: "nginx", tag: "latest" } │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                  Daemon Adapter                          │
│  - Receive gRPC request                                  │
│  - Convert to domain types                               │
│  - Call ImageService.pull()                              │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│              ImageService (Core Logic)                   │
│  1. Parse reference: docker.io/library/nginx:latest      │
│  2. Create RegistryClient                                │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                 RegistryClient                           │
│  1. Authenticate with Docker Hub                         │
│     - GET /v2/library/nginx/manifests/latest (401)       │
│     - GET auth.docker.io/token?scope=... (200 + token)   │
│     - GET /v2/library/nginx/manifests/latest (200)       │
│                                                          │
│  2. Resolve manifest for platform (linux/arm64)          │
│     - If manifest list, find platform-specific digest    │
│     - Fetch platform manifest                            │
│                                                          │
│  3. Download config blob                                 │
│     - GET /v2/library/nginx/blobs/sha256:config...       │
│     - Returns: Image config JSON                         │
│                                                          │
│  4. Download layer blobs (parallel)                      │
│     - GET /v2/library/nginx/blobs/sha256:layer1...       │
│     - GET /v2/library/nginx/blobs/sha256:layer2...       │
│     - GET /v2/library/nginx/blobs/sha256:layer3...       │
│     - Returns: Compressed tar.gz blobs                   │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                  FileSystemStore                         │
│  For each blob:                                          │
│    1. Compute SHA-256 digest                             │
│    2. Verify against expected digest                     │
│    3. Write to blobs/sha256/{hash}                       │
│    4. Write metadata to .meta file                       │
│                                                          │
│  For manifest:                                           │
│    1. Compute digest                                     │
│    2. Write to manifests/sha256/{hash}                   │
│    3. Write metadata                                     │
│                                                          │
│  For tag:                                                │
│    1. Write tag reference to tags/library/nginx/latest   │
│    2. Points to manifest digest                          │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                  OverlaySnapshotter                      │
│  For each layer (sequential):                            │
│    1. Create active snapshot: layer{n}-extract           │
│    2. Get compressed blob from store                     │
│    3. Decompress gzip stream                             │
│    4. Extract tar entries to snapshot fs/                │
│    5. Handle whiteout markers                            │
│    6. Commit snapshot: sha256:layer{n}...                │
│                                                          │
│  Result: Layered snapshots ready for use                 │
│    sha256:layer1... → sha256:layer2... → sha256:layer3...│
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                 ImageService (cont'd)                    │
│  Stream progress events back:                            │
│    - "Resolving"                                         │
│    - "Pulling config"                                    │
│    - "Downloading layer 1/3"                             │
│    - "Extracting layers"                                 │
│    - "Complete"                                          │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                  Daemon Adapter                          │
│  Convert progress to gRPC messages                       │
│  Stream PullImageProgress messages                       │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                    gRPC Layer                            │
│  Stream PullImageProgress to client                      │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────┐
│   CLI    │ Display progress to user
└──────────┘
```

**Data Transformation Points**:

1. **CLI → gRPC**: User input to protocol buffer messages
2. **gRPC → Domain**: Protocol buffer to Rust structs
3. **Registry → Bytes**: HTTP JSON/binary to raw bytes
4. **Bytes → Store**: Raw bytes to content-addressed files
5. **Store → Snapshotter**: Compressed blobs to extracted filesystems
6. **Domain → gRPC**: Rust structs to protocol buffer messages
7. **gRPC → CLI**: Protocol buffer messages to display output

## Container Run Data Flow

```
┌──────────┐
│   CLI    │
└──────────┘
     │ ross-cli container run -it nginx:latest
     ▼
┌──────────────────────────────────────────────────────────┐
│                  Daemon (gRPC)                           │
│  RunInteractive (bidirectional stream)                   │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│             ContainerService.create()                    │
│  1. Resolve image: nginx:latest                          │
│     - Lookup tag in store                                │
│     - Get manifest digest                                │
│     - Parse manifest                                     │
│     - Extract image config                               │
│     - Find top layer: sha256:layer3...                   │
│                                                          │
│  2. Merge container config                               │
│     - Image defaults (entrypoint, cmd, env)              │
│     - User overrides                                     │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                  OverlaySnapshotter                      │
│  1. Create container snapshot                            │
│     key: container-<uuid>                                │
│     parent: sha256:layer3...                             │
│                                                          │
│  2. Build parent chain                                   │
│     container → layer3 → layer2 → layer1                 │
│                                                          │
│  3. Generate mount specification                         │
│     type: overlay                                        │
│     options:                                             │
│       lowerdir=layer3/fs:layer2/fs:layer1/fs            │
│       upperdir=container-<uuid>/fs                       │
│       workdir=container-<uuid>/work                      │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                   RuncShim                               │
│  1. Generate container ID: abc123...                     │
│                                                          │
│  2. Create bundle directory                              │
│     containers/abc123/bundle/                            │
│       ├── rootfs/ (mount point)                          │
│       └── config.json (to be generated)                  │
│                                                          │
│  3. Mount overlay filesystem                             │
│     mount -t overlay overlay \                           │
│       -o lowerdir=...,upperdir=...,workdir=... \         │
│       containers/abc123/bundle/rootfs                    │
│                                                          │
│  4. Generate OCI spec                                    │
│     {                                                    │
│       "process": {                                       │
│         "terminal": true,                                │
│         "args": ["/docker-entrypoint.sh", "nginx", ...], │
│         "env": ["PATH=...", ...],                        │
│         ...                                              │
│       },                                                 │
│       "root": { "path": "rootfs" },                      │
│       "mounts": [ ... ],                                 │
│       "linux": { "namespaces": [ ... ] }                 │
│     }                                                    │
│                                                          │
│  5. Write config.json                                    │
│                                                          │
│  6. Save container metadata                              │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│          ContainerService.run_interactive()              │
│  1. Setup bidirectional channels                         │
│     - input_tx/rx: Client → Container                    │
│     - output_tx/rx: Container → Client                   │
│                                                          │
│  2. Call Shim.run_interactive()                          │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│           RuncShim.run_interactive()                     │
│  1. Create Unix socket for console                       │
│     bundle/console.sock                                  │
│                                                          │
│  2. Start runc with console socket                       │
│     runc run \                                           │
│       --console-socket bundle/console.sock \             │
│       --bundle bundle/ \                                 │
│       abc123                                             │
│                                                          │
│  3. Receive PTY master file descriptor                   │
│     - runc connects to console socket                    │
│     - Passes PTY master FD via SCM_RIGHTS                │
│                                                          │
│  4. Set PTY to raw mode                                  │
│     tcgetattr(), cfmakeraw(), tcsetattr()                │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                      runc                                │
│  1. Clone namespaces                                     │
│     - PID, Network, IPC, UTS, Mount                      │
│                                                          │
│  2. Set up cgroups                                       │
│                                                          │
│  3. Setup rootfs mounts                                  │
│     - Bind mount /proc, /dev, /sys, etc.                 │
│                                                          │
│  4. Pivot root to container rootfs                       │
│                                                          │
│  5. Set resource limits                                  │
│                                                          │
│  6. Set UID/GID                                          │
│                                                          │
│  7. Open PTY slave                                       │
│     - Set as stdin/stdout/stderr                         │
│                                                          │
│  8. Exec container process                               │
│     execve("/docker-entrypoint.sh", args, env)           │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│              Container Process                           │
│  /docker-entrypoint.sh starts nginx                      │
│                                                          │
│  Input:  PTY slave ← PTY master ← Shim ← Client         │
│  Output: PTY slave → PTY master → Shim → Client         │
└──────────────────────────────────────────────────────────┘

Data Flow During Interaction:
═══════════════════════════════════

User Input Path:
──────────────

User types "ls -la" + Enter
     │
     ▼
CLI captures raw terminal bytes
     │
     ▼
gRPC stream: InteractiveInput { stdin: bytes }
     │
     ▼
Daemon forwards to ContainerService
     │
     ▼
ContainerService forwards to Shim
     │
     ▼
Shim writes to PTY master
     │
     ▼
Kernel copies to PTY slave
     │
     ▼
Container process reads from stdin (PTY slave)
     │
     ▼
Shell executes command

Output Path:
───────────

Command produces output
     │
     ▼
Process writes to stdout (PTY slave)
     │
     ▼
Kernel copies to PTY master
     │
     ▼
Shim reads from PTY master
     │
     ▼
OutputEvent::Stdout(bytes)
     │
     ▼
ContainerService streams to Daemon
     │
     ▼
gRPC stream: InteractiveOutput { data: bytes }
     │
     ▼
CLI receives and displays
```

## Container Logs Data Flow

```
┌──────────┐
│   CLI    │
└──────────┘
     │ ross-cli container logs abc123 --follow
     ▼
┌──────────────────────────────────────────────────────────┐
│              Daemon (gRPC)                               │
│  GetLogs (server streaming)                              │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│           ContainerService.get_logs()                    │
│  Create stream from log files                            │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│                 RuncShim                                 │
│  Read log files:                                         │
│    - containers/abc123/bundle/stdout.log                 │
│    - containers/abc123/bundle/stderr.log                 │
│                                                          │
│  If --follow:                                            │
│    - Tail files continuously                             │
│    - Watch for new data                                  │
│    - Stream as LogEntry events                           │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│           ContainerService (cont'd)                      │
│  Stream LogEntry messages:                               │
│    {                                                     │
│      timestamp: 2024-01-15T10:30:00Z,                    │
│      stream: "stdout",                                   │
│      message: "Container log line..."                    │
│    }                                                     │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────┐
│              Daemon (gRPC)                               │
│  Convert and stream to client                            │
└──────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────┐
│   CLI    │ Display logs to user
└──────────┘
```

## Storage Write Path

```
┌─────────────────┐
│  Component      │
│  (ImageService, │
│   Snapshotter)  │
└─────────────────┘
        │
        │ put_blob(media_type, data, expected_digest)
        ▼
┌──────────────────────────────────────────────────────────┐
│              FileSystemStore                             │
│                                                          │
│  1. Compute Digest                                       │
│     ┌────────────────────────────────────┐              │
│     │  hasher = SHA256::new()            │              │
│     │  hasher.update(data)               │              │
│     │  hash = hex::encode(               │              │
│     │           hasher.finalize())       │              │
│     │  digest = Digest {                 │              │
│     │    algorithm: "sha256",            │              │
│     │    hash: hash                      │              │
│     │  }                                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  2. Verify Digest (if expected provided)                │
│     ┌────────────────────────────────────┐              │
│     │  if expected.hash != digest.hash { │              │
│     │    return DigestMismatch;          │              │
│     │  }                                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  3. Write Blob Data                                     │
│     ┌────────────────────────────────────┐              │
│     │  path = blobs/sha256/{hash}        │              │
│     │  create_dir_all(parent)            │              │
│     │  write(path, data)                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  4. Write Metadata                                      │
│     ┌────────────────────────────────────┐              │
│     │  meta = BlobMetadata {             │              │
│     │    media_type,                     │              │
│     │    size: data.len(),               │              │
│     │    created_at: now(),              │              │
│     │    accessed_at: now()              │              │
│     │  }                                 │              │
│     │  meta_path = blobs/sha256/         │              │
│     │                {hash}.meta         │              │
│     │  write(meta_path,                  │              │
│     │        json(meta))                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  5. Return (digest, size)                               │
└──────────────────────────────────────────────────────────┘
        │
        ▼
┌───────────────────────┐
│  Filesystem           │
│                       │
│  blobs/sha256/        │
│    abc123...          │ ← Blob content
│    abc123....meta     │ ← Metadata JSON
└───────────────────────┘
```

## Storage Read Path

```
┌─────────────────┐
│  Component      │
└─────────────────┘
        │
        │ get_blob(digest, offset, length)
        ▼
┌──────────────────────────────────────────────────────────┐
│              FileSystemStore                             │
│                                                          │
│  1. Construct Path                                       │
│     ┌────────────────────────────────────┐              │
│     │  path = blobs/{algo}/{hash}        │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  2. Check Existence                                     │
│     ┌────────────────────────────────────┐              │
│     │  if !path.exists() {               │              │
│     │    return BlobNotFound;            │              │
│     │  }                                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  3. Open File                                           │
│     ┌────────────────────────────────────┐              │
│     │  file = File::open(path)           │              │
│     │  file_size = metadata.len()        │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  4. Seek to Offset                                      │
│     ┌────────────────────────────────────┐              │
│     │  if offset > 0 {                   │              │
│     │    file.seek(Start(offset))        │              │
│     │  }                                 │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  5. Read Data                                           │
│     ┌────────────────────────────────────┐              │
│     │  read_len = if length <= 0 {       │              │
│     │    file_size - offset              │              │
│     │  } else {                          │              │
│     │    length                          │              │
│     │  }                                 │              │
│     │  buffer = vec![0; read_len]        │              │
│     │  file.read_exact(&mut buffer)      │              │
│     └────────────────────────────────────┘              │
│                                                          │
│  6. Return Data                                         │
└──────────────────────────────────────────────────────────┘
        │
        ▼
┌─────────────────┐
│  Vec<u8>        │ Blob content
└─────────────────┘
```

## Memory Flow During Image Pull

```
1. Registry Response (HTTP)
   ──────────────────────────
   Size: 50 MB (compressed layer)
   Location: Network buffer → Tokio buffer

2. Store.put_blob()
   ────────────────
   Size: 50 MB
   Location: Memory → Filesystem
   Operation: Single write

3. Snapshotter.extract_layer()
   ────────────────────────────
   Size: 50 MB (compressed) → 150 MB (uncompressed)
   
   a. Read from store
      Size: 50 MB
      Location: Filesystem → Memory (streaming)
   
   b. Decompress
      Size: Streaming (4 KB chunks)
      Location: Memory → Memory
   
   c. Extract tar
      Size: Per-file (streaming)
      Location: Memory → Filesystem
      
   Result: ~50 MB peak memory usage (streaming)

Total Peak Memory: ~50 MB per layer
(Parallel downloads use semaphore to limit concurrent memory)
```

## Disk I/O Patterns

### Sequential I/O (Efficient)

```
Image Pull:
  Write blobs      → Sequential write to new files
  Extract layers   → Sequential write of extracted files
  
Container Start:
  Read OCI spec    → Single file read
  Mount overlay    → No I/O (kernel operation)
  
Container Run:
  Read/write logs  → Sequential append
```

### Random I/O (Less Efficient)

```
Multiple concurrent operations:
  - Pull multiple images simultaneously
  - Start multiple containers
  - Each touches different files

Optimization:
  - Use SSD storage for better random I/O
  - Limit concurrent pulls (semaphore)
  - Layer sharing reduces total I/O
```

## Network Traffic Patterns

### Image Pull

```
Manifest: ~5 KB
Config: ~7 KB
Layers: 10-100 MB each

Example for nginx:latest:
├─ Manifest:     5 KB
├─ Config:       7 KB
├─ Layer 1:     26 MB  (base OS)
├─ Layer 2:     15 MB  (dependencies)
└─ Layer 3:      2 MB  (nginx files)
Total:          48 MB

With layer sharing:
- First pull: 48 MB
- Second similar image: ~10 MB (only unique layers)
```

### Container Logs

```
Follow mode:
  - Streams continuously
  - Chunks of ~4 KB
  - ~1-10 KB/s typical
  - Spikes during busy activity
```

### Interactive Session

```
Terminal I/O:
  - Bidirectional
  - Small messages (<1 KB typically)
  - Low latency critical
  - ~1-10 KB/s keyboard input
  - ~10-100 KB/s terminal output
```

## Concurrency Patterns

### Image Pull Concurrency

```
Concurrent Operations:
├─ Registry requests (parallel)
│  ├─ Semaphore limit: 3 simultaneous downloads
│  └─ Each download: ~5-10 MB/s
│
├─ Store writes (parallel)
│  └─ Different files, no contention
│
└─ Layer extraction (sequential)
   └─ One layer at a time per image
```

### Multiple Clients

```
Client A: Pull image
Client B: Start container
Client C: View logs

All concurrent:
├─ Separate gRPC streams
├─ No shared mutable state
└─ Async I/O prevents blocking
```

## Data Persistence

### Persistent Data

```
/var/lib/ross/
├── blobs/          Persistent until GC
├── manifests/      Persistent until GC
├── tags/           Updated on push/pull
├── snapshots/      Persistent, layer-shared
└── containers/     Removed on container delete
    └── {id}/
        ├── bundle/
        │   ├── rootfs/     Ephemeral (unmounted)
        │   └── config.json Ephemeral
        └── metadata.json   Ephemeral
```

### Ephemeral Data

```
Container runtime:
├── PTY buffers     In-memory only
├── Process state   Kernel, not persisted
└── Network state   Kernel, recreated
```

## Error Propagation

```
Error Flow:
──────────

runc failure
    │
    ▼
Shim catches error
    │
    ▼
ShimError variant
    │
    ▼
ContainerService maps to ContainerError
    │
    ▼
Daemon adapter converts to gRPC Status
    │
    ▼
gRPC Status code
    │
    ▼
CLI displays user-friendly message
```

**Example**:
```
runc: "container not running"
  → ShimError::ContainerNotRunning
  → ContainerError::InvalidState
  → Status::failed_precondition()
  → CLI: "Error: Container is not running"
```

## Performance Metrics

### Typical Timings

```
Operation               Time
─────────────────────────────────────────
Image pull (nginx)      10-30 seconds
  - Manifest resolve    100-500 ms
  - Config download     50-200 ms
  - Layers download     5-15 seconds
  - Layer extraction    3-10 seconds

Container create        50-200 ms
  - Snapshot prepare    10-50 ms
  - Bundle setup        20-100 ms
  - OCI spec gen        10-30 ms
  - Metadata save       10-20 ms

Container start         100-500 ms
  - runc execution      50-300 ms
  - Process spawn       50-200 ms

Container stop          0.5-10 seconds
  - SIGTERM sent        1 ms
  - Graceful shutdown   0-10 seconds
  - SIGKILL (if needed) 1 ms
```

### Throughput

```
Operations/second:
- Container create: ~20-50/s
- Container start: ~10-20/s  (limited by runc)
- Log streaming: ~1000 lines/s
- gRPC requests: ~5000/s (simple operations)
```

These metrics depend heavily on:
- Disk speed (SSD vs HDD)
- Network bandwidth
- CPU cores
- System load
