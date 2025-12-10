# Content-Addressable Storage

This document describes the storage architecture used by Ross for managing container images and artifacts.

## Storage Philosophy

Ross uses **content-addressable storage (CAS)** where artifacts are stored and referenced by their cryptographic digest. This provides:

1. **Deduplication**: Identical content is stored once, regardless of how many images reference it
2. **Integrity**: Content can be verified by recomputing its digest
3. **Immutability**: Content cannot be changed without changing its address
4. **Sharing**: Multiple images can safely share layers

## Storage Architecture

```
FileSystemStore
├── Blobs (content-addressed)
├── Manifests (content-addressed)
├── Indexes (multi-platform images)
└── Tags (human-readable references)
```

## Digest Format

All artifacts use SHA-256 digests:

```
Format: algorithm:hash
Example: sha256:d4ff818577bc193b309b355b02ebc9220427090057b54a59e73b79bdfe139b83

Components:
- algorithm: "sha256" (fixed)
- hash: 64 hexadecimal characters (256 bits)
```

## Directory Structure

```
/var/lib/ross/
├── blobs/
│   └── sha256/
│       ├── d4ff818577bc...                    # Blob content
│       ├── d4ff818577bc....meta               # Blob metadata
│       ├── e3b0c44298fc...
│       └── e3b0c44298fc....meta
│
├── manifests/
│   └── sha256/
│       ├── a1b2c3d4e5f6...                    # Manifest JSON
│       ├── a1b2c3d4e5f6....meta               # Manifest metadata
│       ├── f6e5d4c3b2a1...
│       └── f6e5d4c3b2a1....meta
│
├── indexes/
│   └── sha256/
│       ├── 1a2b3c4d5e6f...                    # Image index JSON
│       └── ...
│
└── tags/
    ├── library/
    │   ├── nginx/
    │   │   ├── latest                         # Tag metadata
    │   │   ├── 1.21
    │   │   └── stable
    │   └── alpine/
    │       ├── latest
    │       └── 3.18
    └── myregistry.com/
        └── myuser/
            └── myapp/
                └── v1.0
```

## Blobs

### What are Blobs?

Blobs are the raw content of:
- **Image layers** (compressed tar.gz files)
- **Image configs** (JSON files describing the image)
- **Other artifacts** (buildinfo, attestations, etc.)

### Blob Storage

**File**: `blobs/{algorithm}/{hash}`
```
Raw binary content of the blob
```

**Metadata**: `blobs/{algorithm}/{hash}.meta`
```json
{
  "media_type": "application/vnd.docker.image.rootfs.diff.tar.gzip",
  "size": 2811969,
  "created_at": 1699564800,
  "accessed_at": 1699564800
}
```

### Media Types for Blobs

| Media Type | Description |
|------------|-------------|
| `application/vnd.docker.image.rootfs.diff.tar.gzip` | Compressed layer |
| `application/vnd.oci.image.layer.v1.tar+gzip` | OCI compressed layer |
| `application/vnd.docker.container.image.v1+json` | Image config |
| `application/vnd.oci.image.config.v1+json` | OCI image config |

### Blob Operations

#### Put Blob

```rust
async fn put_blob(
    &self,
    media_type: &str,
    data: &[u8],
    expected_digest: Option<&Digest>,
) -> Result<(Digest, i64), StoreError>
```

**Process**:
```
1. Compute SHA-256 hash of data
   ├─> hasher = SHA256::new()
   ├─> hasher.update(data)
   └─> hash = hex::encode(hasher.finalize())

2. Verify expected digest (if provided)
   ├─> expected.hash == computed.hash?
   └─> Error if mismatch

3. Create storage path
   ├─> path = blobs/sha256/{hash}
   └─> Create parent directories

4. Write blob content
   └─> fs::write(path, data)

5. Write metadata
   ├─> meta = BlobMetadata {
   │     media_type, size, created_at, accessed_at
   │   }
   ├─> meta_path = blobs/sha256/{hash}.meta
   └─> fs::write(meta_path, serde_json::to_string(&meta))

6. Return (digest, size)
```

#### Get Blob (with Range Support)

```rust
async fn get_blob(
    &self,
    digest: &Digest,
    offset: i64,
    length: i64,  // -1 = read to end
) -> Result<Vec<u8>, StoreError>
```

**Process**:
```
1. Construct path
   └─> path = blobs/sha256/{hash}

2. Check existence
   └─> Error if not found

3. Open file
   └─> file = fs::File::open(path)

4. Seek to offset (if > 0)
   └─> file.seek(SeekFrom::Start(offset))

5. Read data
   ├─> read_len = if length <= 0 {
   │     file_size - offset
   │   } else {
   │     length
   │   }
   └─> file.read_exact(&mut buffer[..read_len])

6. Return data
```

**Use Cases for Range Reads**:
- Streaming large blobs
- Resuming interrupted downloads
- Partial content inspection

#### Stat Blob

```rust
async fn stat_blob(&self, digest: &Digest) 
    -> Result<Option<BlobInfo>, StoreError>
```

Returns metadata without reading content:
```rust
BlobInfo {
    digest: Some(Digest { algorithm: "sha256", hash: "..." }),
    size: 2811969,
    media_type: "application/vnd.docker.image.rootfs.diff.tar.gzip",
    created_at: Some(Timestamp { seconds: 1699564800, nanos: 0 }),
    accessed_at: Some(Timestamp { seconds: 1699564800, nanos: 0 }),
}
```

#### Delete Blob

```rust
async fn delete_blob(&self, digest: &Digest) -> Result<bool, StoreError>
```

**Process**:
```
1. Check if blob exists
2. Remove blob file
3. Remove metadata file
4. Return true if deleted, false if not found
```

**Warning**: Deleting blobs should only be done during garbage collection when no manifests reference them.

## Manifests

### What are Manifests?

Manifests describe the structure of an image:
- List of layers (by digest)
- Reference to config (by digest)
- Metadata (architecture, OS, etc.)

### Manifest Storage

**File**: `manifests/{algorithm}/{hash}`
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": {
    "mediaType": "application/vnd.docker.container.image.v1+json",
    "size": 7023,
    "digest": "sha256:config-hash..."
  },
  "layers": [
    {
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 2811969,
      "digest": "sha256:layer1-hash..."
    },
    {
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 1572864,
      "digest": "sha256:layer2-hash..."
    }
  ]
}
```

**Metadata**: `manifests/{algorithm}/{hash}.meta`
```json
{
  "media_type": "application/vnd.docker.distribution.manifest.v2+json",
  "size": 1234,
  "created_at": 1699564800,
  "schema_version": "2"
}
```

### Manifest Types

#### Docker Manifest V2 Schema 2
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": { ... },
  "layers": [ ... ]
}
```

#### OCI Image Manifest
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": { ... },
  "layers": [ ... ]
}
```

#### Manifest List (Multi-Platform)
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
  "manifests": [
    {
      "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
      "size": 1234,
      "digest": "sha256:amd64-manifest...",
      "platform": {
        "architecture": "amd64",
        "os": "linux"
      }
    },
    {
      "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
      "size": 1235,
      "digest": "sha256:arm64-manifest...",
      "platform": {
        "architecture": "arm64",
        "os": "linux"
      }
    }
  ]
}
```

### Manifest Operations

#### Put Manifest

```rust
async fn put_manifest(
    &self,
    content: &[u8],
    media_type: &str,
) -> Result<(Digest, i64), StoreError>
```

Similar to put_blob but for manifests:
1. Hash the manifest content
2. Write to `manifests/{algorithm}/{hash}`
3. Write metadata
4. Return digest

#### Get Manifest

```rust
async fn get_manifest(&self, digest: &Digest) 
    -> Result<(Vec<u8>, String), StoreError>
```

Returns:
- Manifest content (JSON bytes)
- Media type from metadata

## Tags

### What are Tags?

Tags provide human-readable names for specific manifest digests:
- `nginx:latest` → `sha256:abc123...`
- `alpine:3.18` → `sha256:def456...`

Tags are **mutable** - they can be updated to point to different manifests.

### Tag Storage

**File**: `tags/{repository}/{tag}`

```json
{
  "digest_algorithm": "sha256",
  "digest_hash": "a1b2c3d4e5f6...",
  "updated_at": 1699564800
}
```

### Repository Structure

Tags are organized by repository:

```
tags/
├── library/              # Official images
│   ├── nginx/
│   │   ├── latest
│   │   ├── 1.21
│   │   ├── 1.21.6
│   │   └── stable
│   └── alpine/
│       ├── latest
│       ├── 3.18
│       └── edge
│
└── myregistry.com/       # External registry
    └── myuser/
        └── myapp/
            ├── latest
            ├── v1.0
            └── dev
```

### Tag Operations

#### Set Tag

```rust
async fn set_tag(
    &self,
    repository: &str,
    tag: &str,
    digest: &Digest,
) -> Result<Option<Digest>, StoreError>
```

**Process**:
```
1. Construct path
   └─> path = tags/{repository}/{tag}

2. Check for existing tag
   ├─> Read existing file if exists
   └─> Parse previous digest

3. Create directories
   └─> fs::create_dir_all(parent)

4. Write new tag metadata
   ├─> meta = TagMetadata {
   │     digest_algorithm, digest_hash, updated_at
   │   }
   └─> fs::write(path, serde_json::to_string(&meta))

5. Return previous digest (if any)
```

#### Resolve Tag

```rust
async fn resolve_tag(
    &self,
    repository: &str,
    tag: &str,
) -> Result<(Digest, String), StoreError>
```

Returns:
- Manifest digest that tag points to
- Media type of the manifest

**Process**:
```
1. Read tag metadata
   └─> path = tags/{repository}/{tag}

2. Parse digest from metadata
   └─> Digest { algorithm, hash }

3. Get manifest media type
   ├─> manifest_path = manifests/{algorithm}/{hash}
   └─> Read media type from manifest metadata

4. Return (digest, media_type)
```

#### Delete Tag

```rust
async fn delete_tag(
    &self,
    repository: &str,
    tag: &str,
) -> Result<bool, StoreError>
```

Removes tag file. The referenced manifest remains until garbage collected.

#### List Tags

```rust
async fn list_tags(&self, repository: &str) 
    -> Result<Vec<TagInfo>, StoreError>
```

Returns all tags for a repository:
```rust
vec![
    TagInfo {
        tag: "latest".to_string(),
        digest: Some(Digest { ... }),
        updated_at: Some(Timestamp { ... }),
    },
    TagInfo {
        tag: "1.21".to_string(),
        digest: Some(Digest { ... }),
        updated_at: Some(Timestamp { ... }),
    },
]
```

## Content Deduplication

### Layer Sharing Example

```
Image: nginx:1.21          Image: nginx:1.22
├─ Layer 1 (base OS)       ├─ Layer 1 (base OS)      ← Shared
│  digest: sha256:aaa...   │  digest: sha256:aaa...
│                           │
├─ Layer 2 (nginx)         ├─ Layer 2 (nginx)        ← Shared
│  digest: sha256:bbb...   │  digest: sha256:bbb...
│                           │
└─ Layer 3 (config 1.21)   ├─ Layer 3 (config 1.22)  ← Different
   digest: sha256:ccc...   │  digest: sha256:ddd...
                            │
                            └─ Layer 4 (new features)  ← Unique
                               digest: sha256:eee...
```

**Storage**:
```
blobs/sha256/
├── aaa... (stored once, used by both)
├── bbb... (stored once, used by both)
├── ccc... (nginx:1.21 only)
├── ddd... (nginx:1.22 only)
└── eee... (nginx:1.22 only)
```

**Benefits**:
- Layer 1 and 2 stored once (~80% of image size)
- Only unique layers downloaded for nginx:1.22
- Significant disk space savings
- Faster pulls when layers are cached

## Garbage Collection

### Purpose

Remove unreferenced artifacts to reclaim disk space.

### Algorithm

```
1. Identify Referenced Digests
   ┌─────────────────────────────────────┐
   │ Scan all tags                       │
   │ ├─> tags/library/nginx/latest      │
   │ │   → sha256:manifest1...           │
   │ ├─> tags/library/alpine/latest     │
   │ │   → sha256:manifest2...           │
   │ └─> ...                             │
   │                                     │
   │ For each manifest:                  │
   │ ├─> Parse manifest JSON             │
   │ ├─> Extract config digest           │
   │ └─> Extract layer digests           │
   │                                     │
   │ Build set of referenced digests:    │
   │ {manifest1, manifest2, config1,     │
   │  layer1, layer2, layer3, ...}       │
   └─────────────────────────────────────┘
                   │
                   ▼
2. Find Unreferenced Artifacts
   ┌─────────────────────────────────────┐
   │ List all manifests                  │
   │ For each manifest:                  │
   │   if digest not in referenced set   │
   │     mark for deletion               │
   │                                     │
   │ List all blobs                      │
   │ For each blob:                      │
   │   if digest not in referenced set   │
   │     mark for deletion               │
   └─────────────────────────────────────┘
                   │
                   ▼
3. Delete Unreferenced Artifacts
   ┌─────────────────────────────────────┐
   │ For each marked artifact:           │
   │ ├─> Delete file                     │
   │ ├─> Delete metadata                 │
   │ └─> Track freed space               │
   │                                     │
   │ Return statistics:                  │
   │ ├─> blobs_removed: 5                │
   │ ├─> manifests_removed: 2            │
   │ └─> bytes_freed: 52428800           │
   └─────────────────────────────────────┘
```

### Garbage Collection API

```rust
async fn garbage_collect(
    &self,
    dry_run: bool,
    delete_untagged: bool,
) -> Result<(i64, i64, i64, Vec<Digest>), StoreError>
```

**Parameters**:
- `dry_run`: If true, report what would be deleted without actually deleting
- `delete_untagged`: If true, delete manifests not referenced by any tag

**Returns**:
- Number of blobs removed
- Number of manifests removed
- Bytes freed
- List of deleted digests

### Example Garbage Collection

**Before**:
```
Tags:
- library/nginx/latest → sha256:new-manifest...

Manifests:
- sha256:new-manifest... (referenced)
- sha256:old-manifest... (unreferenced)

Blobs:
- sha256:layer1... (referenced by new)
- sha256:layer2... (referenced by new)
- sha256:layer3... (only referenced by old)
- sha256:old-config... (only referenced by old)
```

**After GC**:
```
Tags:
- library/nginx/latest → sha256:new-manifest...

Manifests:
- sha256:new-manifest... (kept)

Blobs:
- sha256:layer1... (kept)
- sha256:layer2... (kept)

Deleted:
- sha256:old-manifest...
- sha256:layer3...
- sha256:old-config...

Space freed: ~10 MB
```

## Store Information

### Get Store Info

```rust
async fn get_store_info(&self) -> Result<(i64, i64, i64, i64), StoreError>
```

Returns:
- Total size (bytes)
- Blob count
- Manifest count
- Tag count

### Example Output

```rust
let (total_size, blob_count, manifest_count, tag_count) = 
    store.get_store_info().await?;

// total_size: 157286400 (150 MB)
// blob_count: 25
// manifest_count: 8
// tag_count: 12
```

## Integrity Verification

### Blob Integrity

Every blob put/get operation verifies integrity:

```rust
// During put_blob
let computed_digest = compute_sha256(data);
if expected_digest.is_some() && computed_digest != expected_digest {
    return Err(StoreError::DigestMismatch {
        expected: format_digest(expected),
        actual: format_digest(&computed_digest),
    });
}
```

### Manifest Chain Verification

Manifest → Config → Layers forms a chain of trust:

```
Manifest (sha256:manifest...)
    │
    ├─> Config (sha256:config...)
    │   └─> Verified when fetched
    │
    └─> Layers
        ├─> Layer 1 (sha256:layer1...)
        │   └─> Verified when fetched
        ├─> Layer 2 (sha256:layer2...)
        │   └─> Verified when fetched
        └─> Layer 3 (sha256:layer3...)
            └─> Verified when fetched
```

Each artifact's digest is verified independently, ensuring the entire image is intact.

## Performance Considerations

### Parallel Operations

Store operations are async and can be parallelized:

```rust
// Download multiple blobs concurrently
let handles: Vec<_> = layers.iter()
    .map(|layer| {
        let store = store.clone();
        let digest = layer.digest.clone();
        tokio::spawn(async move {
            let data = download_blob(&digest).await?;
            store.put_blob(media_type, &data, Some(&digest)).await
        })
    })
    .collect();

// Wait for all to complete
let results = futures::future::join_all(handles).await;
```

### Caching

The store does not implement caching - this is intentional:
- File system already provides page cache
- Keep implementation simple
- Avoid memory overhead
- Trust OS-level optimizations

### Disk Usage

Monitor disk usage with store info:

```rust
let (total_size, _, _, _) = store.get_store_info().await?;
let threshold = 10 * 1024 * 1024 * 1024; // 10 GB

if total_size > threshold {
    store.garbage_collect(false, true).await?;
}
```

## Error Handling

### Common Errors

```rust
pub enum StoreError {
    BlobNotFound(String),
    ManifestNotFound(String),
    TagNotFound(String, String),
    DigestMismatch { expected: String, actual: String },
    Io(std::io::Error),
    Serialization(serde_json::Error),
}
```

### Error Recovery

Most errors are non-recoverable and require user intervention:

- **BlobNotFound**: Re-pull the image
- **DigestMismatch**: Re-download the blob (corrupted data)
- **Io**: Check disk space, permissions
- **Serialization**: Metadata corruption, re-pull image

## Store Maintenance

### Regular Tasks

1. **Garbage Collection**: Run periodically (e.g., weekly)
   ```rust
   store.garbage_collect(false, true).await?;
   ```

2. **Disk Usage Monitoring**: Alert when space is low
   ```rust
   let (total_size, _, _, _) = store.get_store_info().await?;
   ```

3. **Integrity Checks**: Verify critical images periodically
   ```rust
   for tag in critical_tags {
       let (digest, _) = store.resolve_tag(&repo, &tag).await?;
       let info = store.stat_blob(&digest).await?;
       assert!(info.is_some(), "Critical blob missing!");
   }
   ```

### Backup Strategy

Content-addressable storage simplifies backups:

```bash
# Backup entire store
tar czf ross-store-backup.tar.gz /var/lib/ross/

# Backup specific images
for tag in nginx:latest alpine:latest; do
  # Export image metadata (would need implementation)
  ross-cli export $tag > ${tag//\//_}.tar
done
```

### Migration

Moving to a new host:

```bash
# On old host
tar czf ross-store.tar.gz /var/lib/ross/

# On new host
tar xzf ross-store.tar.gz -C /var/lib/
systemctl start ross-daemon
```

All content-addressed artifacts remain valid after migration!
