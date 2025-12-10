# Container Lifecycle

This document describes the complete lifecycle of a container in Ross, from creation to removal.

## Container States

```
┌─────────┐    start     ┌─────────┐    pause     ┌────────┐
│ Created │ ──────────>  │ Running │ ──────────>  │ Paused │
└─────────┘              └─────────┘              └────────┘
     │                        │                        │
     │ remove                 │ stop                   │ unpause
     │                        │                        │
     ▼                        ▼                        ▼
┌─────────┐              ┌─────────┐              ┌─────────┐
│ Deleted │              │ Stopped │              │ Running │
└─────────┘              └─────────┘              └─────────┘
                              │
                              │ remove
                              ▼
                         ┌─────────┐
                         │ Deleted │
                         └─────────┘
```

## Create Container

### High-Level Flow

```
┌──────┐    gRPC    ┌────────┐         ┌─────────────────┐
│ CLI  │ ────────>  │ Daemon │ ─────>  │ ContainerService│
└──────┘            └────────┘         └─────────────────┘
                                              │
                                              ▼
                        ┌────────────────────────────────┐
                        │    1. Resolve Image            │
                        │    2. Prepare Snapshot         │
                        │    3. Generate OCI Spec        │
                        │    4. Create Container         │
                        └────────────────────────────────┘
                                              │
                                              ▼
                        ┌────────────────────────────────┐
                        │       Container ID             │
                        │       State: Created           │
                        └────────────────────────────────┘
```

### Detailed Steps

#### 1. Parse and Resolve Image

```rust
// Input: "nginx:latest"
let (repository, tag) = parse_image_reference("nginx:latest");
// repository: "library/nginx"
// tag: "latest"

// Lookup in store
let tags = store.list_tags("library/nginx").await?;
let tag_info = tags.find(|t| t.tag == "latest")?;
let manifest_digest = tag_info.digest;

// Get manifest
let (manifest_bytes, media_type) = store.get_manifest(&manifest_digest).await?;
let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;

// Get image config
let config_digest = manifest.config.digest;
let config_bytes = store.get_blob(&config_digest, 0, -1).await?;
let image_config: ImageConfig = serde_json::from_slice(&config_bytes)?;
```

**Extracted Information**:

```rust
ImageConfigInfo {
    top_layer: Some("sha256:layer3..."),
    entrypoint: vec!["/docker-entrypoint.sh"],
    cmd: vec!["nginx", "-g", "daemon off;"],
    env: vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "NGINX_VERSION=1.21.6",
    ],
    working_dir: "/",
    user: "",
}
```

#### 2. Prepare Snapshot

```
┌────────────────────────────────────────────────────────────┐
│                    Snapshot Preparation                    │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  Top Layer: sha256:layer3...                               │
│                                                            │
│  Snapshotter.prepare(                                      │
│    key: "container-<uuid>",                                │
│    parent: "sha256:layer3...",                             │
│    labels: {                                               │
│      "container": "true",                                  │
│      "image": "nginx:latest"                               │
│    }                                                       │
│  )                                                         │
│                                                            │
│  Creates:                                                  │
│    snapshots/container-<uuid>/                             │
│      ├── fs/           (empty, will be upper dir)          │
│      ├── work/         (overlay work dir)                  │
│      └── metadata.json                                     │
│                                                            │
│  Returns mount specification:                              │
│    type: "overlay"                                         │
│    source: "overlay"                                       │
│    options: [                                              │
│      "lowerdir=/snapshots/layer3/fs:                       │
│                /snapshots/layer2/fs:                       │
│                /snapshots/layer1/fs",                      │
│      "upperdir=/snapshots/container-<uuid>/fs",            │
│      "workdir=/snapshots/container-<uuid>/work"            │
│    ]                                                       │
└────────────────────────────────────────────────────────────┘
```

**Overlay Mount Structure**:

```
container-<uuid> filesystem view:
┌──────────────────────────────────────────┐
│         Container Writable Layer         │  ← upperdir
│  (modifications: new files, changes)     │
├──────────────────────────────────────────┤
│            Layer 3 (top)                 │  ┐
│         /app/nginx.conf                  │  │
├──────────────────────────────────────────┤  │ lowerdir
│            Layer 2                       │  │ (read-only)
│         /usr/local/nginx/...             │  │
├──────────────────────────────────────────┤  │
│            Layer 1 (base)                │  │
│         /bin, /etc, /usr, ...            │  │
└──────────────────────────────────────────┘  ┘
```

#### 3. Merge Container Configuration

```rust
// User config takes precedence over image defaults

let entrypoint = if user_config.entrypoint.is_empty() {
    image_config.entrypoint    // ["/docker-entrypoint.sh"]
} else {
    user_config.entrypoint.clone()
};

let cmd = if user_config.cmd.is_empty() {
    image_config.cmd           // ["nginx", "-g", "daemon off;"]
} else {
    user_config.cmd.clone()
};

let env = if user_config.env.is_empty() {
    image_config.env           // ["PATH=...", "NGINX_VERSION=..."]
} else {
    let mut merged = image_config.env;
    merged.extend(user_config.env.clone());  // User vars override
    merged
};

let working_dir = user_config.working_dir.or(image_config.working_dir);
let user = user_config.user.or(image_config.user);
```

**Merged Configuration Example**:

```rust
ContainerConfig {
    image: "nginx:latest",
    entrypoint: vec!["/docker-entrypoint.sh"],
    cmd: vec!["nginx", "-g", "daemon off;"],
    env: vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "NGINX_VERSION=1.21.6",
        "MY_VAR=custom",  // User-provided
    ],
    working_dir: Some("/"),
    user: Some("nginx"),
    tty: false,
    open_stdin: false,
    // ...
}
```

#### 4. Create Container via Shim

```
┌────────────────────────────────────────────────────────────┐
│                   Shim.create()                            │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  1. Generate Container ID                                  │
│     id = uuid::new_v4()  // "abc123..."                    │
│                                                            │
│  2. Create Bundle Directory                                │
│     containers/abc123/bundle/                              │
│       ├── rootfs/         (mount point)                    │
│       └── config.json     (OCI spec, to be generated)      │
│                                                            │
│  3. Mount Overlay Filesystem                               │
│     mount -t overlay overlay \                             │
│       -o lowerdir=...,upperdir=...,workdir=... \           │
│       containers/abc123/bundle/rootfs                      │
│                                                            │
│  4. Generate OCI Runtime Spec                              │
│     (see OCI Spec Generation section)                      │
│                                                            │
│  5. Write config.json                                      │
│     containers/abc123/bundle/config.json                   │
│                                                            │
│  6. Save Container Metadata                                │
│     containers/abc123/metadata.json                        │
│                                                            │
│  7. Return Container ID                                    │
│     → "abc123..."                                          │
└────────────────────────────────────────────────────────────┘
```

### OCI Spec Generation

The shim generates an OCI runtime specification:

```json
{
  "ociVersion": "1.0.2",
  "process": {
    "terminal": false,
    "user": {
      "uid": 101,
      "gid": 101
    },
    "args": ["/docker-entrypoint.sh", "nginx", "-g", "daemon off;"],
    "env": [
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      "NGINX_VERSION=1.21.6"
    ],
    "cwd": "/",
    "noNewPrivileges": true
  },
  "root": {
    "path": "rootfs",
    "readonly": false
  },
  "hostname": "container-abc123",
  "mounts": [
    {
      "destination": "/proc",
      "type": "proc",
      "source": "proc"
    },
    {
      "destination": "/dev",
      "type": "tmpfs",
      "source": "tmpfs",
      "options": ["nosuid", "strictatime", "mode=755", "size=65536k"]
    },
    {
      "destination": "/dev/pts",
      "type": "devpts",
      "source": "devpts",
      "options": [
        "nosuid",
        "noexec",
        "newinstance",
        "ptmxmode=0666",
        "mode=0620"
      ]
    },
    {
      "destination": "/dev/shm",
      "type": "tmpfs",
      "source": "shm",
      "options": ["nosuid", "noexec", "nodev", "mode=1777", "size=65536k"]
    },
    {
      "destination": "/sys",
      "type": "sysfs",
      "source": "sysfs",
      "options": ["nosuid", "noexec", "nodev", "ro"]
    }
  ],
  "linux": {
    "namespaces": [
      { "type": "pid" },
      { "type": "network" },
      { "type": "ipc" },
      { "type": "uts" },
      { "type": "mount" }
    ]
  }
}
```

### Container Creation Result

```
Container Created!
├── ID: abc123...
├── State: Created
├── Bundle: /var/lib/ross/containers/abc123/bundle/
│   ├── config.json (OCI spec)
│   └── rootfs/ (mounted overlay)
├── Snapshot: container-<uuid>
└── Ready to start
```

## Start Container

### Flow

```
┌──────┐    gRPC    ┌────────┐         ┌─────────────────┐
│ CLI  │ ────────> │ Daemon │ ─────>   │ ContainerService│
└──────┘            └────────┘         └─────────────────┘
                                              │
                                              ▼
                                       ┌─────────────┐
                                       │    Shim     │
                                       └─────────────┘
                                              │
                                              ▼
                                       ┌─────────────┐
                                       │    runc     │
                                       └─────────────┘
```

### Detailed Steps

```
┌────────────────────────────────────────────────────────────┐
│                    Shim.start()                            │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  1. Verify Container State                                 │
│     current_state == Created? ✓                            │
│                                                            │
│  2. Update Container State                                 │
│     state = Running                                        │
│     started_at = now()                                     │
│     Save metadata                                          │
│                                                            │
│  3. Execute runc                                           │
│     cmd: runc --root /var/lib/ross/runc \                  │
│               run \                                        │
│               --bundle /var/lib/ross/containers/abc123/bundle \
│               --pid-file container.pid \                   │
│               --no-pivot \                                 │
│               --detach \                                   │
│               abc123                                       │
│                                                            │
│  4. Capture Output (detached mode)                         │
│     stdout → containers/abc123/bundle/stdout.log           │
│     stderr → containers/abc123/bundle/stderr.log           │
│                                                            │
│  5. Read PID                                               │
│     pid = read(container.pid)                              │
│     Save to metadata                                       │
│                                                            │
│  6. Container Running                                      │
│     state = Running                                        │
│     pid = 12345                                            │
└────────────────────────────────────────────────────────────┘
```

### runc Execution

```
runc run (detached mode)
    │
    ├─> Clone namespaces (PID, NET, IPC, UTS, MOUNT)
    │
    ├─> Set up cgroups
    │
    ├─> Mount rootfs and propagate mounts
    │
    ├─> Pivot root to container rootfs
    │
    ├─> Set resource limits
    │
    ├─> Set UID/GID
    │
    ├─> Exec container process
    │     (entrypoint + cmd)
    │
    └─> Return to caller (detached)
        Container continues running in background
```

## Run Interactive (`docker run -it` equivalent)

### Flow Diagram

```
┌─────┐              ┌────────┐              ┌──────────────┐
│ CLI │ ──stdin──>   │ Daemon │ ──stdin──>   │ ContainerSvc │
│     │              │        │              │              │
│     │ <─stdout───  │        │ <─stdout──   │              │
└─────┘              └────────┘              └──────────────┘
                                                    │
                                                    ▼
                                             ┌────────────┐
                                             │    Shim    │
                                             └────────────┘
                                                    │
                                                    ▼
                                             ┌────────────┐
                                             │  PTY       │
                                             │  Master/   │
                                             │  Slave     │
                                             └────────────┘
                                                    │
                                                    ▼
                                             ┌────────────┐
                                             │   runc     │
                                             │ (console)  │
                                             └────────────┘
```

### PTY Setup

```
┌────────────────────────────────────────────────────────────┐
│            Interactive Container Setup                     │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  1. Create Unix Socket for Console                         │
│     socket_path = bundle/console.sock                      │
│     listener = UnixListener::bind(socket_path)             │
│                                                            │
│  2. Start runc with console-socket                         │
│     runc run \                                             │
│       --console-socket bundle/console.sock \               │
│       --bundle bundle/ \                                   │
│       container-id                                         │
│                                                            │
│  3. Accept Connection and Receive PTY FD                   │
│     (stream, _) = listener.accept()                        │
│     pty_master_fd = receive_fd_via_socket(stream)          │
│                                                            │
│  4. Set PTY to Raw Mode                                    │
│     tcgetattr(pty_master_fd)                               │
│     cfmakeraw(&mut termios)                                │
│     tcsetattr(pty_master_fd, termios)                      │
│                                                            │
│  5. Bidirectional Copy                                     │
│     ┌──────────────────────────────────────┐               │
│     │  Client stdin ──> PTY Master         │               │
│     │  PTY Master   ──> Client stdout      │               │
│     └──────────────────────────────────────┘               │
│                                                            │
│  6. Handle Special Events                                  │
│     - Terminal resize (SIGWINCH)                           │
│     - Detach keys (Ctrl+P, Ctrl+Q)                         │
│     - Process exit                                         │
└────────────────────────────────────────────────────────────┘
```

### Input/Output Events

```rust
pub enum InputEvent {
    Stdin(Vec<u8>),           // Raw terminal input
    Resize {
        width: u32,
        height: u32
    },                         // Terminal size change
}

pub enum OutputEvent {
    Stdout(Vec<u8>),          // Raw terminal output
    Stderr(Vec<u8>),          // Error output
    Exit(WaitResult),         // Process exit
}
```

### Interactive Session Flow

```
User types "ls -la" + Enter
    │
    ▼
CLI captures raw bytes: [108, 115, 32, 45, 108, 97, 13]
    │
    ▼
gRPC stream: InputEvent::Stdin(bytes)
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
PTY slave (in container) receives input
    │
    ▼
Shell executes "ls -la"
    │
    ▼
Output written to PTY slave
    │
    ▼
PTY master receives output
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
Daemon streams via gRPC
    │
    ▼
CLI displays output to user
```

## Stop Container

### Flow

```
┌────────────────────────────────────────────────────────────┐
│                    Shim.stop()                             │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  1. Verify Container State                                 │
│     current_state == Running? ✓                            │
│                                                            │
│  2. Send SIGTERM                                           │
│     runc kill container-id 15                              │
│                                                            │
│  3. Wait for Graceful Shutdown                             │
│     sleep(timeout)  // default: 10 seconds                 │
│                                                            │
│  4. Check if Still Running                                 │
│     runc state container-id                                │
│                                                            │
│  5. Force Kill if Necessary                                │
│     if still_running {                                     │
│       runc kill container-id 9  // SIGKILL                 │
│     }                                                      │
│                                                            │
│  6. Update Container State                                 │
│     state = Stopped                                        │
│     finished_at = now()                                    │
│     pid = None                                             │
│     Save metadata                                          │
└────────────────────────────────────────────────────────────┘
```

### Signal Flow

```
SIGTERM (15) - Graceful Shutdown
    │
    ├─> Process signal handler runs
    │
    ├─> Cleanup: close files, flush buffers, save state
    │
    └─> Process exits
        │
        ├─> Exit code captured
        │
        └─> Container state → Stopped

If timeout expires:

SIGKILL (9) - Forced Termination
    │
    ├─> Kernel immediately terminates process
    │
    ├─> No cleanup possible
    │
    └─> Process exits
        │
        └─> Container state → Stopped
```

## Remove Container

### Flow

```
┌────────────────────────────────────────────────────────────┐
│                    Shim.delete()                           │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  1. Check Container State                                  │
│     if state == Running && !force {                        │
│       error("Container is running")                        │
│     }                                                      │
│                                                            │
│  2. Stop Container if Running (force mode)                 │
│     if state == Running && force {                         │
│       stop(container_id, timeout)                          │
│     }                                                      │
│                                                            │
│  3. Delete from runc                                       │
│     runc delete --force container-id                       │
│                                                            │
│  4. Unmount Rootfs                                         │
│     umount containers/abc123/bundle/rootfs                 │
│                                                            │
│  5. Remove Snapshot                                        │
│     snapshotter.remove("container-<uuid>")                 │
│     - Removes overlay snapshot                             │
│     - Cleans up fs/ and work/ directories                  │
│                                                            │
│  6. Remove Container Directory                             │
│     rm -rf containers/abc123/                              │
│     - Bundle removed                                       │
│     - Logs removed                                         │
│     - Metadata removed                                     │
│                                                            │
│  7. Remove from Internal Tracking                          │
│     containers.remove(container_id)                        │
└────────────────────────────────────────────────────────────┘
```

## Container Logs

### Log Capture

For detached containers:

```
runc run --detach \
  1> containers/abc123/bundle/stdout.log \
  2> containers/abc123/bundle/stderr.log
```

For streaming output:

```rust
pub fn run_streaming(&self, id: String)
    -> impl Stream<Item = Result<OutputEvent, ShimError>>
{
    // Spawn runc with stdout/stderr piped
    let mut child = Command::new("runc")
        .arg("run")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Stream output in real-time
    stream! {
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        loop {
            select! {
                Ok(n) = stdout.read(&mut buf) => {
                    if n > 0 {
                        yield OutputEvent::Stdout(buf[..n].to_vec());
                    }
                }
                Ok(n) = stderr.read(&mut buf) => {
                    if n > 0 {
                        yield OutputEvent::Stderr(buf[..n].to_vec());
                    }
                }
                Ok(status) = child.wait() => {
                    yield OutputEvent::Exit(WaitResult {
                        exit_code: status.code().unwrap_or(-1),
                        error: None,
                    });
                    break;
                }
            }
        }
    }
}
```

## Container Inspection

Returns detailed container information:

```json
{
  "container": {
    "id": "abc123...",
    "names": ["/my-container"],
    "image": "nginx:latest",
    "state": "running",
    "created": "2024-01-15T10:30:00Z"
  },
  "state": {
    "status": "running",
    "running": true,
    "paused": false,
    "pid": 12345,
    "exit_code": 0,
    "started_at": "2024-01-15T10:30:05Z"
  },
  "config": {
    "image": "nginx:latest",
    "cmd": ["nginx", "-g", "daemon off;"],
    "entrypoint": ["/docker-entrypoint.sh"],
    "env": ["PATH=...", "NGINX_VERSION=..."],
    "working_dir": "/"
  },
  "host_config": {
    "binds": ["/host/path:/container/path"],
    "network_mode": "bridge",
    "privileged": false
  }
}
```

## Complete Lifecycle Diagram

```
    create
       │
       ▼
┌─────────────┐
│   Created   │ ──────────────────┐
└─────────────┘                   │
       │                          │
       │ start                    │ remove
       ▼                          │
┌─────────────┐                   │
│   Running   │ ◄─────┐           │
└─────────────┘       │           │
   │   │   │          │ unpause   │
   │   │   │          │           │
   │   │   │    ┌─────────────┐   │
   │   │   │    │   Paused    │   │
   │   │   │    └─────────────┘   │
   │   │   │          ▲           │
   │   │   │          │           │
   │   │   │          │ pause     │
   │   │   └──────────┘           │
   │   │                          │
   │   │ stop                     │
   │   │                          │
   │   │ kill                     │
   │   │                          │
   │   ▼                          │
   │ ┌─────────────┐              │
   │ │   Stopped   │ ─────────────┤
   │ └─────────────┘              │
   │                              │
   │ wait (exit)                  │
   │                              │
   ▼                              ▼
┌─────────────┐              ┌──────────┐
│   Exited    │              │ Deleted  │
└─────────────┘              └──────────┘
       │
       │ remove
       ▼
┌─────────────┐
│   Deleted   │
└─────────────┘
```

## Resource Cleanup Matrix

| Resource       | Created | Started | Stopped | Removed |
| -------------- | ------- | ------- | ------- | ------- |
| Container ID   | ✓       | ✓       | ✓       | ✗       |
| Metadata       | ✓       | ✓       | ✓       | ✗       |
| Bundle Dir     | ✓       | ✓       | ✓       | ✗       |
| OCI Spec       | ✓       | ✓       | ✓       | ✗       |
| Snapshot       | ✓       | ✓       | ✓       | ✗       |
| Rootfs Mount   | ✓       | ✓       | ✗       | ✗       |
| runc Container | ✗       | ✓       | ✗       | ✗       |
| Process (PID)  | ✗       | ✓       | ✗       | ✗       |
| Logs           | ✗       | ✓       | ✓       | ✗       |

✓ = Exists, ✗ = Removed/Cleaned up
