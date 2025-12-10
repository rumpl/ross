# Snapshotter Architecture

This document describes the overlay snapshotter used by Ross for efficient filesystem layer management.

## Overview

The snapshotter manages filesystem snapshots using OverlayFS, providing:
- **Layered filesystems** from image layers
- **Copy-on-write semantics** for container modifications
- **Efficient storage** through layer sharing
- **Fast snapshot creation** (no data copying)

## Core Concepts

### Snapshot Types

```
┌────────────────┐
│     View       │  Read-only snapshot for inspection
└────────────────┘

┌────────────────┐
│    Active      │  Writable snapshot being prepared
└────────────────┘

┌────────────────┐
│   Committed    │  Immutable finalized snapshot
└────────────────┘
```

**View**:
- Read-only access to a snapshot
- No upper/work directories
- Used for inspection without modification

**Active**:
- Writable snapshot being prepared
- Has upper and work directories
- Can be committed to become permanent

**Committed**:
- Immutable snapshot
- Cannot be modified
- Can be used as parent for new snapshots

### Snapshot Lifecycle

```
   prepare()
       │
       ▼
┌─────────────┐
│   Active    │
└─────────────┘
       │
       │ commit()
       ▼
┌─────────────┐
│  Committed  │
└─────────────┘
       │
       │ prepare() with parent
       ▼
┌─────────────┐
│   Active    │  (new snapshot)
└─────────────┘
```

## Directory Structure

```
/var/lib/ross/snapshots/
├── sha256:layer1.../
│   ├── fs/              # Filesystem content
│   ├── work/            # OverlayFS work directory
│   └── metadata.json    # Snapshot metadata
│
├── sha256:layer2.../
│   ├── fs/
│   ├── work/
│   └── metadata.json
│
├── sha256:layer3.../
│   ├── fs/
│   ├── work/
│   └── metadata.json
│
└── container-abc123.../
    ├── fs/              # Container's writable layer
    ├── work/            # OverlayFS work directory
    └── metadata.json
```

### Metadata Format

```json
{
  "info": {
    "key": "sha256:layer1...",
    "parent": null,
    "kind": "Committed",
    "created_at": 1699564800,
    "updated_at": 1699564800,
    "labels": {
      "containerd.io/snapshot/layer.digest": "sha256:layer1..."
    }
  }
}
```

## OverlayFS Primer

### How OverlayFS Works

```
┌─────────────────────────────────────────────────────────┐
│                    Merged View                          │
│  (What the container sees as its root filesystem)       │
└─────────────────────────────────────────────────────────┘
                        ▲
                        │
        ┌───────────────┼───────────────┐
        │               │               │
┌───────────────┐ ┌─────────────┐ ┌────────────────┐
│  Upper Dir    │ │  Work Dir   │ │   Lower Dirs   │
│ (read-write)  │ │  (scratch)  │ │  (read-only)   │
└───────────────┘ └─────────────┘ └────────────────┘
│  New files    │ │  Temp files │ │  Layer 3       │
│  Modified     │ │  for atomic │ │  Layer 2       │
│  files        │ │  operations │ │  Layer 1       │
└───────────────┘ └─────────────┘ └────────────────┘
```

**Upper Directory**:
- Writable layer on top
- New files created here
- Modified files copied here (copy-on-write)
- Deleted files marked with whiteout files

**Work Directory**:
- Temporary directory for atomic operations
- Must be on same filesystem as upper
- Used by kernel for preparing changes

**Lower Directories**:
- One or more read-only layers
- Stacked from top to bottom (rightmost is bottom)
- Files from upper layers hide lower layers
- Original layer content never modified

### OverlayFS Example

**Layers**:
```
Layer 1 (base):
├── bin/
│   ├── sh
│   └── ls
└── etc/
    └── passwd

Layer 2:
├── etc/
│   └── nginx/
│       └── nginx.conf
└── usr/
    └── sbin/
        └── nginx

Upper (container writes):
├── etc/
│   └── nginx/
│       └── nginx.conf  (modified)
└── var/
    └── log/
        └── nginx/
            └── access.log (new file)
```

**Merged View**:
```
/
├── bin/              (from Layer 1)
│   ├── sh
│   └── ls
├── etc/              (merged)
│   ├── passwd        (from Layer 1)
│   └── nginx/
│       └── nginx.conf (from Upper - modified)
├── usr/              (from Layer 2)
│   └── sbin/
│       └── nginx
└── var/              (from Upper)
    └── log/
        └── nginx/
            └── access.log
```

## Snapshot Operations

### Prepare

Creates a writable snapshot for container use.

```rust
async fn prepare(
    &self,
    key: &str,                    // "container-abc123"
    parent: Option<&str>,         // "sha256:layer3..."
    labels: HashMap<String, String>,
) -> Result<Vec<Mount>, SnapshotterError>
```

**Process**:
```
1. Validate parent exists (if specified)
   └─> parent must be Committed snapshot

2. Create snapshot directories
   ├─> snapshots/container-abc123/
   │   ├── fs/      (empty, will be upperdir)
   │   └── work/    (empty, overlayfs workdir)

3. Build parent chain
   └─> Traverse parent links to build layer list
       container-abc123 → layer3 → layer2 → layer1

4. Generate mount specification
   └─> Mount {
         type: "overlay",
         source: "overlay",
         options: [
           "lowerdir=layer3/fs:layer2/fs:layer1/fs",
           "upperdir=container-abc123/fs",
           "workdir=container-abc123/work"
         ]
       }

5. Save metadata
   └─> snapshots/container-abc123/metadata.json
       {
         "key": "container-abc123",
         "parent": "sha256:layer3...",
         "kind": "Active",
         "labels": {...}
       }

6. Return mount specification
   └─> Consumer will perform the actual mount
```

### View

Creates a read-only snapshot for inspection.

```rust
async fn view(
    &self,
    key: &str,
    parent: Option<&str>,
    labels: HashMap<String, String>,
) -> Result<Vec<Mount>, SnapshotterError>
```

**Differences from Prepare**:
- No upperdir or workdir created
- Mount is read-only
- Kind is "View" instead of "Active"

**Mount Specification**:
```rust
Mount {
    type: "overlay",
    source: "overlay",
    options: [
        "lowerdir=layer3/fs:layer2/fs:layer1/fs"
    ]
}
```

### Commit

Converts active snapshot to committed (immutable).

```rust
async fn commit(
    &self,
    key: &str,              // "sha256:layer3..."
    active_key: &str,       // "sha256:layer3-extract"
    labels: HashMap<String, String>,
) -> Result<(), SnapshotterError>
```

**Process**:
```
1. Verify active snapshot exists and is Active
   └─> Error if not found or wrong kind

2. Rename snapshot directory
   ├─> FROM: snapshots/sha256:layer3-extract/
   └─> TO:   snapshots/sha256:layer3.../

3. Update metadata
   ├─> key: "sha256:layer3..."
   ├─> kind: Committed
   ├─> updated_at: now
   └─> Merge labels

4. Remove old key from tracking
   └─> Remove "sha256:layer3-extract" entry

5. Add new key to tracking
   └─> Add "sha256:layer3..." entry
```

### Remove

Deletes a snapshot.

```rust
async fn remove(&self, key: &str) -> Result<(), SnapshotterError>
```

**Process**:
```
1. Check for dependent snapshots
   └─> Error if any snapshot has this as parent

2. Remove snapshot directory
   └─> rm -rf snapshots/{key}/

3. Remove from tracking
   └─> Remove entry from in-memory map
```

**Important**: Cannot remove snapshot if it has children.

### Mounts

Returns mount specification for existing snapshot.

```rust
async fn mounts(&self, key: &str) -> Result<Vec<Mount>, SnapshotterError>
```

Reconstructs mount specification from snapshot metadata and parent chain.

### Stat

Returns information about a snapshot.

```rust
async fn stat(&self, key: &str) -> Result<SnapshotInfo, SnapshotterError>
```

Returns:
```rust
SnapshotInfo {
    key: "container-abc123".to_string(),
    parent: Some("sha256:layer3...".to_string()),
    kind: SnapshotKind::Active,
    created_at: 1699564800,
    updated_at: 1699564800,
    labels: HashMap::new(),
}
```

### List

Lists all snapshots, optionally filtered by parent.

```rust
async fn list(&self, parent_filter: Option<&str>) 
    -> Result<Vec<SnapshotInfo>, SnapshotterError>
```

**Example**:
```rust
// List all snapshots with layer2 as parent
let children = snapshotter.list(Some("sha256:layer2...")).await?;
// Returns: [SnapshotInfo for layer3, SnapshotInfo for container-xyz, ...]
```

### Usage

Calculates disk usage of a snapshot.

```rust
async fn usage(&self, key: &str) -> Result<Usage, SnapshotterError>
```

Returns:
```rust
Usage {
    size: 52428800,   // Bytes used
    inodes: 1250,     // Number of inodes
}
```

**Note**: Only counts files in the snapshot's fs/ directory, not inherited from parents.

## Layer Extraction

### Extract Layer

Extracts a compressed layer tarball into a snapshot.

```rust
async fn extract_layer(
    &self,
    digest: &str,              // "sha256:layer1..."
    parent_key: Option<&str>,  // None for base layer
    key: &str,                 // "sha256:layer1..."
    labels: HashMap<String, String>,
) -> Result<(String, i64), SnapshotterError>
```

**Process**:

```
┌─────────────────────────────────────────────────────────┐
│                   Extract Layer Flow                     │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  1. Fetch Compressed Blob                               │
│     ├─> store.get_blob("sha256:layer1...", 0, -1)       │
│     └─> Returns gzip-compressed tar bytes               │
│                                                          │
│  2. Create Temporary Active Snapshot                    │
│     ├─> prepare("sha256:layer1-extract", parent, {})    │
│     └─> Creates writable snapshot                       │
│                                                          │
│  3. Extract Tar Archive                                 │
│     ├─> Decompress gzip stream                          │
│     ├─> Read tar entries                                │
│     ├─> For each entry:                                 │
│     │   ├─> Check for whiteout (.wh.*)                  │
│     │   ├─> Extract to fs/ directory                    │
│     │   └─> Preserve permissions, ownership             │
│     └─> Track total bytes extracted                     │
│                                                          │
│  4. Handle Whiteouts                                    │
│     (see Whiteout Handling section)                     │
│                                                          │
│  5. Commit Snapshot                                     │
│     ├─> commit("sha256:layer1...",                      │
│     │           "sha256:layer1-extract",                │
│     │           labels)                                 │
│     └─> Snapshot becomes immutable                      │
│                                                          │
│  6. Return Result                                       │
│     └─> (key: "sha256:layer1...", size: 2811969)       │
└─────────────────────────────────────────────────────────┘
```

### Whiteout Handling

OCI layers use **whiteout files** to indicate deleted files:

```
File: .wh.{filename}
Meaning: {filename} should be deleted/hidden
```

**Examples**:
- `.wh.tmp` → Hide/delete `/tmp`
- `etc/.wh.nginx.conf` → Hide/delete `/etc/nginx.conf`
- `app/.wh.old-binary` → Hide/delete `/app/old-binary`

**Processing**:
```rust
for entry in tar_archive.entries()? {
    let mut entry = entry?;
    let path = entry.path()?;
    
    if let Some(name) = path.file_name() {
        let name_str = name.to_string_lossy();
        
        if name_str.starts_with(".wh.") {
            // This is a whiteout marker
            let original_name = name_str.strip_prefix(".wh.").unwrap();
            let parent_dir = path.parent().unwrap_or(Path::new(""));
            let target = extract_dir.join(parent_dir).join(original_name);
            
            // Remove the actual file/directory
            if target.exists() {
                if target.is_dir() {
                    fs::remove_dir_all(&target)?;
                } else {
                    fs::remove_file(&target)?;
                }
            }
            
            // Don't extract the whiteout marker itself
            continue;
        }
    }
    
    // Normal file - extract it
    entry.unpack_in(extract_dir)?;
}
```

### Extraction Example

**Layer Tarball Contents**:
```
etc/
  nginx/
    nginx.conf
  .wh.old-config    # Whiteout marker
usr/
  local/
    bin/
      nginx
var/
  log/
    nginx/
```

**After Extraction**:
```
snapshots/sha256:layer2.../fs/
├── etc/
│   └── nginx/
│       └── nginx.conf
├── usr/
│   └── local/
│       └── bin/
│           └── nginx
└── var/
    └── log/
        └── nginx/

Note: etc/old-config is NOT present (removed by whiteout)
      .wh.old-config marker is NOT extracted
```

## Parent Chain

### Building the Chain

Snapshots can have parent relationships forming a chain:

```
container-abc123 → sha256:layer3... → sha256:layer2... → sha256:layer1... → (none)
```

**Code**:
```rust
fn get_parent_chain(&self, key: &str) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = Some(key.to_string());
    
    while let Some(k) = current {
        if let Some(info) = self.snapshots.get(&k) {
            chain.push(k);
            current = info.parent.clone();
        } else {
            break;
        }
    }
    
    chain
}
```

**Example**:
```rust
let chain = snapshotter.get_parent_chain("container-abc123");
// Returns: ["container-abc123", "sha256:layer3...", "sha256:layer2...", "sha256:layer1..."]
```

### Using the Chain

The parent chain determines the overlay lowerdir order:

```bash
# For container-abc123 with parent chain above:
mount -t overlay overlay \
  -o lowerdir=sha256:layer3.../fs:sha256:layer2.../fs:sha256:layer1.../fs,\
     upperdir=container-abc123/fs,\
     workdir=container-abc123/work \
  /target/path
```

**Important**: Layers are listed left-to-right (top to bottom), where:
- Leftmost = topmost layer (highest priority)
- Rightmost = base layer (lowest priority)

## Mount Specifications

### Single Layer (No Parent)

```rust
Mount {
    mount_type: "bind",
    source: "/var/lib/ross/snapshots/sha256:layer1.../fs",
    target: "",  // Set by caller
    options: vec!["rw", "rbind"],
}
```

**Explanation**: Simple bind mount for base layer.

### Layered (With Parents)

```rust
Mount {
    mount_type: "overlay",
    source: "overlay",
    target: "",  // Set by caller
    options: vec![
        "lowerdir=/var/lib/ross/snapshots/sha256:layer3.../fs:\
                  /var/lib/ross/snapshots/sha256:layer2.../fs:\
                  /var/lib/ross/snapshots/sha256:layer1.../fs",
        "upperdir=/var/lib/ross/snapshots/container-abc123/fs",
        "workdir=/var/lib/ross/snapshots/container-abc123/work",
    ],
}
```

**Explanation**: Full overlay with multiple lower layers and writable upper.

### Read-Only Layered

```rust
Mount {
    mount_type: "overlay",
    source: "overlay",
    target: "",
    options: vec![
        "lowerdir=/var/lib/ross/snapshots/sha256:layer3.../fs:\
                  /var/lib/ross/snapshots/sha256:layer2.../fs:\
                  /var/lib/ross/snapshots/sha256:layer1.../fs",
    ],
}
```

**Explanation**: No upperdir/workdir = read-only view.

## Performance Characteristics

### Space Efficiency

**Without Snapshotter**:
```
Image A (100 MB) + Image B (110 MB) = 210 MB
(80 MB shared layers duplicated)
```

**With Snapshotter**:
```
Base layers: 80 MB (shared)
Image A unique: 20 MB
Image B unique: 30 MB
Total: 130 MB (38% savings)
```

### Time Efficiency

**Creating Snapshot**:
- No data copying
- Just metadata creation
- O(1) time complexity
- ~10ms typical

**Mounting Overlay**:
- Kernel operation
- No data transfer
- O(n) in number of layers
- ~50ms for 10 layers

**Layer Extraction**:
- Must decompress and extract tar
- I/O bound operation
- Depends on layer size
- ~2-5 seconds for 50MB layer

## Error Handling

### Common Errors

```rust
pub enum SnapshotterError {
    AlreadyExists(String),
    NotFound(String),
    ParentNotFound(String),
    HasDependents(String),
    InvalidState { expected: String, actual: String },
    ExtractionFailed(String),
    Io(std::io::Error),
    Store(ross_store::StoreError),
}
```

### Error Scenarios

**AlreadyExists**:
```
Trying to create snapshot with existing key
→ Use unique key or check existence first
```

**ParentNotFound**:
```
Parent snapshot doesn't exist
→ Ensure parent is extracted/committed first
```

**HasDependents**:
```
Trying to delete snapshot with children
→ Remove children first, or use force
```

**ExtractionFailed**:
```
Corrupt tar archive or I/O error during extraction
→ Re-download layer, check disk space
```

## Cleanup and Maintenance

### Cleanup Orphaned Snapshots

```rust
async fn cleanup(&self) -> Result<i64, SnapshotterError>
```

Removes snapshot directories that:
- Have no metadata file
- Are not tracked in memory
- Were created but never completed

**Process**:
```
1. List all directories in snapshots/
2. For each directory:
   - Check if tracked in memory
   - If not tracked:
     - Calculate size
     - Delete directory
     - Add to freed space
3. Return total bytes reclaimed
```

### Snapshot Lifecycle Management

**Best Practices**:

1. **Extract layers immediately after pull**
   ```rust
   for layer in layers {
       snapshotter.extract_layer(&layer.digest, parent, &key, labels).await?;
   }
   ```

2. **Remove container snapshots on delete**
   ```rust
   snapshotter.remove(&container_snapshot_key).await?;
   ```

3. **Keep layer snapshots until image removed**
   - Layer snapshots are shared
   - Only remove when no images reference them

4. **Run cleanup periodically**
   ```rust
   tokio::spawn(async move {
       loop {
           tokio::time::sleep(Duration::from_secs(3600)).await;
           if let Err(e) = snapshotter.cleanup().await {
               tracing::error!("Cleanup failed: {}", e);
           }
       }
   });
   ```

## Integration with Container Lifecycle

### Container Creation

```
1. Identify top layer: sha256:layer3...
2. Create container snapshot: container-abc123
   ├─> parent: sha256:layer3...
   └─> kind: Active
3. Get mount specification
4. Mount overlay filesystem
5. Container ready to start
```

### Container Running

```
Container modifies filesystem:
├─> New files created in upperdir
├─> Modified files copied to upperdir (CoW)
└─> Deleted files marked with whiteouts
```

### Container Removal

```
1. Unmount overlay filesystem
2. Remove container snapshot: container-abc123
   ├─> Deletes upperdir (container changes)
   └─> workdir cleaned up
3. Layer snapshots remain (shared)
```

## Debugging

### Inspect Snapshot

```rust
let info = snapshotter.stat("container-abc123").await?;
println!("Key: {}", info.key);
println!("Parent: {:?}", info.parent);
println!("Kind: {}", info.kind);
println!("Created: {}", info.created_at);
println!("Labels: {:?}", info.labels);
```

### Check Mount

```bash
# List all overlay mounts
mount | grep overlay

# Inspect specific mount
findmnt /var/lib/ross/containers/abc123/bundle/rootfs

# Check overlay options
cat /proc/self/mountinfo | grep overlay
```

### Verify Layer Content

```bash
# List files in layer
ls -laR /var/lib/ross/snapshots/sha256:layer1.../fs/

# Check layer size
du -sh /var/lib/ross/snapshots/sha256:layer1.../fs/
```

## Advanced Topics

### Lazy Extraction

Future optimization: Extract layers on-demand instead of upfront.

**Benefits**:
- Faster image pulls
- Use less disk space
- Only extract what's accessed

**Challenges**:
- More complex implementation
- Requires FUSE or kernel support
- Performance overhead on first access

### Snapshot Deduplication

Future optimization: Deduplicate identical files across snapshots.

**Approach**:
- Use hardlinks for identical files
- Content-addressed filesystem
- Reduces disk usage significantly

**Challenges**:
- Breaks CoW semantics
- Requires careful permission handling
- Increased metadata overhead
