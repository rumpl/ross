# Image Pulling Architecture

This document describes the flow and architecture of pulling container images in Ross.

## High-Level Flow

```
┌─────────┐         ┌────────┐         ┌──────────────┐         ┌──────────┐
│   CLI   │ ─────> │ Daemon │ ─────> │ ImageService │ ─────> │ Registry │
└─────────┘  gRPC   └────────┘         └──────────────┘  HTTP   └──────────┘
                                              │
                                              │ Save artifacts
                                              ▼
                                       ┌────────────┐
                                       │   Store    │
                                       └────────────┘
                                              │
                                              │ Extract layers
                                              ▼
                                       ┌──────────────┐
                                       │ Snapshotter  │
                                       └──────────────┘
```

## Detailed Pull Flow

### Phase 1: Reference Resolution

```
User Input: "nginx:latest"
     │
     ▼
┌──────────────────────────────┐
│  ImageReference::parse()     │
│                              │
│  registry: docker.io         │
│  repository: library/nginx   │
│  tag: latest                 │
└──────────────────────────────┘
```

**Reference Parsing Rules**:
- `nginx` → `docker.io/library/nginx:latest`
- `nginx:1.21` → `docker.io/library/nginx:1.21`
- `myuser/myapp` → `docker.io/myuser/myapp:latest`
- `gcr.io/project/image:v1` → `gcr.io/project/image:v1`
- `localhost:5000/app` → `localhost:5000/app:latest`

### Phase 2: Registry Authentication

```
┌────────────────────────────────────────────────────────────────┐
│ 1. Initial Request (No Auth)                                   │
│    GET /v2/library/nginx/manifests/latest                      │
│                                                                 │
│    ← 401 Unauthorized                                           │
│      WWW-Authenticate: Bearer realm="...", service="...",       │
│                        scope="repository:library/nginx:pull"    │
└────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────────┐
│ 2. Token Request                                                │
│    GET {realm}?service={service}&scope={scope}                 │
│                                                                 │
│    ← 200 OK                                                     │
│      {"token": "eyJhbGc..."}                                    │
└────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────────┐
│ 3. Authenticated Request                                        │
│    GET /v2/library/nginx/manifests/latest                      │
│    Authorization: Bearer eyJhbGc...                             │
│                                                                 │
│    ← 200 OK (Manifest)                                          │
└────────────────────────────────────────────────────────────────┘
```

### Phase 3: Manifest Resolution

#### Single Platform Image

```
GET /v2/library/nginx/manifests/latest
Accept: application/vnd.docker.distribution.manifest.v2+json

← 200 OK
  Content-Type: application/vnd.docker.distribution.manifest.v2+json
  Docker-Content-Digest: sha256:abc123...

{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": {
    "digest": "sha256:config123...",
    "mediaType": "application/vnd.docker.container.image.v1+json",
    "size": 7023
  },
  "layers": [
    {
      "digest": "sha256:layer1...",
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 2811969
    },
    {
      "digest": "sha256:layer2...",
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 1572864
    }
  ]
}
```

#### Multi-Platform Image

```
GET /v2/library/nginx/manifests/latest
Accept: application/vnd.docker.distribution.manifest.list.v2+json

← 200 OK (Manifest List)
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
  "manifests": [
    {
      "digest": "sha256:manifest-amd64...",
      "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
      "platform": {
        "architecture": "amd64",
        "os": "linux"
      }
    },
    {
      "digest": "sha256:manifest-arm64...",
      "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
      "platform": {
        "architecture": "arm64",
        "os": "linux"
      }
    }
  ]
}
```

Then fetch platform-specific manifest:
```
GET /v2/library/nginx/manifests/sha256:manifest-arm64...
```

### Phase 4: Config Download

```
┌────────────────────────────────────────────────────────────────┐
│                        Config Download                          │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Digest: sha256:config123...                                   │
│  Progress: "Pulling config"                                    │
│                                                                 │
│  GET /v2/library/nginx/blobs/sha256:config123...               │
│  Authorization: Bearer ...                                      │
│                                                                 │
│  ← 200 OK (Config JSON)                                         │
│                                                                 │
│  Store → put_blob(media_type, data, expected_digest)           │
│           ✓ Hash verified                                       │
│           ✓ Saved to blobs/sha256/config123...                 │
│                                                                 │
│  Progress: "Pull complete"                                     │
└────────────────────────────────────────────────────────────────┘
```

**Image Config Structure**:
```json
{
  "architecture": "arm64",
  "os": "linux",
  "config": {
    "Env": ["PATH=/usr/local/bin:/usr/bin"],
    "Cmd": ["nginx", "-g", "daemon off;"],
    "WorkingDir": "/",
    "ExposedPorts": {"80/tcp": {}},
    "Labels": {"version": "1.21"}
  },
  "rootfs": {
    "type": "layers",
    "diff_ids": [
      "sha256:layer1-uncompressed...",
      "sha256:layer2-uncompressed..."
    ]
  },
  "history": [...]
}
```

### Phase 5: Layer Downloads (Concurrent)

```
┌──────────────────────────────────────────────────────────────────┐
│                    Concurrent Layer Downloads                     │
│                   (max_concurrent_downloads = 3)                  │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │   Layer 1       │  │   Layer 2       │  │   Layer 3       │ │
│  │                 │  │                 │  │                 │ │
│  │ Status: DL      │  │ Status: DL      │  │ Status: Wait    │ │
│  │ [==========>  ] │  │ [======>      ] │  │ [...........] │ │
│  │ 70% (2.1/3 MB)  │  │ 50% (0.8/1.6M)  │  │ Queued        │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
│                                                                   │
│  Semaphore: [Permit 1] [Permit 2] [Available: 1]                │
└──────────────────────────────────────────────────────────────────┘
```

**Layer Download State Machine**:
```
┌─────────────┐
│   Queued    │
└─────────────┘
      │
      │ Acquire semaphore
      ▼
┌─────────────┐
│ Downloading │ ─────> "Downloading [1/5]"
└─────────────┘
      │
      │ Download complete
      ▼
┌─────────────┐
│ Downloaded  │ ─────> "Download complete"
└─────────────┘
      │
      │ Store blob
      ▼
┌─────────────┐
│   Stored    │ ─────> "Pull complete"
└─────────────┘
      │
      │ Release semaphore
      ▼
```

**Layer Download Implementation**:
```rust
async fn download_layer(
    registry: Arc<RegistryClient>,
    store: Arc<FileSystemStore>,
    reference: ImageReference,
    layer: Descriptor,
    index: usize,
    total: usize,
    semaphore: Arc<Semaphore>,
    tx: mpsc::Sender<LayerEvent>,
) {
    // Check if already exists
    if store.has_blob(&layer.digest).await {
        tx.send(LayerEvent::Exists { id: layer.digest }).await;
        return;
    }
    
    // Acquire semaphore (blocks if at limit)
    let _permit = semaphore.acquire().await.unwrap();
    
    tx.send(LayerEvent::Downloading { id, index, total }).await;
    
    // Download layer bytes
    let bytes = registry.get_blob_bytes(&reference, &layer.digest).await?;
    
    tx.send(LayerEvent::Downloaded { id }).await;
    
    // Store with verification
    store.put_blob(&layer.media_type, &bytes, Some(&layer.digest)).await?;
    
    tx.send(LayerEvent::Stored { id }).await;
}
```

### Phase 6: Manifest Storage

```
┌────────────────────────────────────────────────────────────────┐
│                       Store Manifest                            │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Serialize manifest to JSON                                    │
│  Hash manifest → sha256:manifest-hash...                        │
│                                                                 │
│  Store → put_manifest(content, media_type)                     │
│           ✓ Saved to manifests/sha256/manifest-hash...         │
│           ✓ Metadata saved                                      │
│                                                                 │
│  Store → set_tag("library/nginx", "latest", digest)            │
│           ✓ Saved to tags/library/nginx/latest                 │
│           ✓ Tag → Manifest mapping complete                    │
└────────────────────────────────────────────────────────────────┘
```

**Tag Metadata**:
```json
{
  "digest_algorithm": "sha256",
  "digest_hash": "manifest-hash...",
  "updated_at": 1699564800
}
```

### Phase 7: Layer Extraction

```
┌──────────────────────────────────────────────────────────────────┐
│                        Layer Extraction                           │
│                      (Sequential, bottom-up)                      │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  Layer 1 (base):                                                 │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ 1. Create active snapshot: layer1-extract                  │ │
│  │ 2. Get blob: sha256:layer1...                              │ │
│  │ 3. Decompress gzip stream                                  │ │
│  │ 4. Extract tar to snapshots/layer1-extract/fs/            │ │
│  │    - /bin/, /etc/, /usr/, /var/, ...                       │ │
│  │ 5. Commit snapshot: sha256:layer1...                       │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              │                                    │
│                              ▼ parent                             │
│  Layer 2:                                                        │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ 1. Create active snapshot: layer2-extract                  │ │
│  │    parent: sha256:layer1...                                │ │
│  │ 2. Get blob: sha256:layer2...                              │ │
│  │ 3. Extract tar to snapshots/layer2-extract/fs/            │ │
│  │    - /app/, /etc/nginx/nginx.conf (overlay)                │ │
│  │    - .wh.tmp (whiteout marker)                             │ │
│  │ 4. Handle whiteouts                                         │ │
│  │ 5. Commit snapshot: sha256:layer2...                       │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              │                                    │
│                              ▼ parent                             │
│  Layer 3:                                                        │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ ... (repeat for each layer)                                │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  Final result: Layered snapshots ready for container use        │
└──────────────────────────────────────────────────────────────────┘
```

**Snapshot Hierarchy After Extraction**:
```
sha256:layer1... (committed)
    │
    └── sha256:layer2... (committed)
            │
            └── sha256:layer3... (committed)  ← Top layer
```

### Phase 8: Progress Reporting

Throughout the pull, progress events stream to the client:

```
Resolving
  status: "Resolving"

Resolved digest: sha256:manifest-hash...
  status: "Resolved digest: sha256:manifest-hash..."

abc123...: Pulling config
  id: "abc123..." (short hash)
  status: "Pulling config"

abc123...: Pull complete
  id: "abc123..."
  status: "Pull complete"

Layer 1: Downloading [1/3]
  id: "def456..." (short hash)
  status: "Downloading"
  progress: "[1/3]"

Layer 1: Download complete
  id: "def456..."
  status: "Download complete"

Layer 1: Pull complete
  id: "def456..."
  status: "Pull complete"

(... repeat for each layer ...)

Extracting layers
  status: "Extracting layers"

def456...: Extracting layer 1/3
  id: "def456..."
  status: "Extracting layer 1/3"

def456...: Extracted (2811969 bytes)
  id: "def456..."
  status: "Extracted (2811969 bytes)"

(... repeat for each layer ...)

Digest: sha256:manifest-hash...
  status: "Digest: sha256:manifest-hash..."

Status: Downloaded newer image for docker.io/library/nginx:latest
  status: "Status: Downloaded newer image for ..."
```

## Error Handling

### Network Errors
```
┌──────────────────────────────────────┐
│ Network timeout or connection error  │
│ ↓                                    │
│ Retry with exponential backoff       │
│ ↓                                    │
│ Max retries exceeded                 │
│ ↓                                    │
│ Error: "Failed to download layer"    │
└──────────────────────────────────────┘
```

### Authentication Errors
```
┌──────────────────────────────────────┐
│ 401 Unauthorized                     │
│ ↓                                    │
│ Parse WWW-Authenticate header        │
│ ↓                                    │
│ Request token from auth endpoint     │
│ ↓                                    │
│ 401 on token request (bad creds)     │
│ ↓                                    │
│ Error: "Authentication failed"       │
└──────────────────────────────────────┘
```

### Digest Mismatch
```
┌──────────────────────────────────────┐
│ Download complete                    │
│ ↓                                    │
│ Compute SHA-256 hash                 │
│ ↓                                    │
│ Compare with expected digest         │
│ ↓                                    │
│ Mismatch detected                    │
│ ↓                                    │
│ Delete partial blob                  │
│ ↓                                    │
│ Error: "Digest mismatch"             │
│   expected: sha256:abc...            │
│   actual:   sha256:def...            │
└──────────────────────────────────────┘
```

### Storage Errors
```
┌──────────────────────────────────────┐
│ Insufficient disk space              │
│ ↓                                    │
│ Write fails during put_blob          │
│ ↓                                    │
│ Cleanup partial files                │
│ ↓                                    │
│ Error: "Insufficient disk space"     │
└──────────────────────────────────────┘
```

## Optimization Strategies

### Already Downloaded Layers
```
┌──────────────────────────────────────┐
│ Check if layer exists in store       │
│ ↓                                    │
│ stat_blob(digest)                    │
│ ↓                                    │
│ Exists?                              │
│ ↓ YES                  ↓ NO          │
│ Skip download          Download      │
│ ↓                      ↓              │
│ "Already exists"       "Downloading" │
└──────────────────────────────────────┘
```

### Layer Sharing
Multiple images can share layers:
```
nginx:1.21        nginx:1.22
     │                 │
     └─── Layer A ─────┘  (shared)
     ├─── Layer B ─────┘  (shared)
     └─── Layer C         (unique)
               └─── Layer D  (unique)
```

Only unique layers are downloaded; shared layers are reused.

### Concurrent Downloads
Semaphore prevents overwhelming:
- Network bandwidth
- Registry rate limits
- Memory usage for buffering

## State Diagram

```
          ┌──────────────┐
          │    Start     │
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Parse Reference│
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Authenticate  │
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Get Manifest  │
          └──────────────┘
                 │
         ┌───────┴───────┐
         │               │
    Manifest V2    Manifest List
         │               │
         │         Resolve Platform
         │               │
         └───────┬───────┘
                 ▼
          ┌──────────────┐
          │Download Config│
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Download Layers│ (parallel)
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Store Manifest│
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │  Set Tag     │
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │Extract Layers│ (sequential)
          └──────────────┘
                 │
                 ▼
          ┌──────────────┐
          │   Complete   │
          └──────────────┘
```

## Storage After Pull

```
/var/lib/ross/
├── blobs/
│   └── sha256/
│       ├── config123...           # Image config
│       ├── config123....meta
│       ├── layer1...              # Compressed layers
│       ├── layer1....meta
│       ├── layer2...
│       └── layer2....meta
├── manifests/
│   └── sha256/
│       ├── manifest-hash...       # Image manifest
│       └── manifest-hash....meta
├── tags/
│   └── library/
│       └── nginx/
│           └── latest             # Tag → Manifest mapping
└── snapshots/
    ├── sha256:layer1.../          # Extracted layers
    │   ├── fs/
    │   └── metadata.json
    ├── sha256:layer2.../
    │   ├── fs/
    │   └── metadata.json
    └── sha256:layer3.../
        ├── fs/
        └── metadata.json
```

The image is now ready to be used for creating containers!
