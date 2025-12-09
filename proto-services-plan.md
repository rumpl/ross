# Docker Protobuf Services Development Plan

## Overview
Create two new protobuf files defining gRPC services for Docker Image and Container management. These services will provide comprehensive APIs for managing Docker images and containers with streaming support where appropriate.

## File Structure
```
proto/
├── ross.proto          (existing)
├── image.proto         (new)
└── container.proto     (new)
```

## Package & Style Conventions
- Follow existing conventions from `ross.proto`
- Use `syntax = "proto3"`
- Package name: `ross` (consistent with existing)
- Naming: PascalCase for messages, snake_case for fields

---

## 1. Image Service (`proto/image.proto`)

### Service Definition: `ImageService`

#### RPC Methods

| Method | Request | Response | Type | Description |
|--------|---------|----------|------|-------------|
| `ListImages` | `ListImagesRequest` | `ListImagesResponse` | Unary | List all images with optional filters |
| `InspectImage` | `InspectImageRequest` | `InspectImageResponse` | Unary | Get detailed image information |
| `PullImage` | `PullImageRequest` | `PullImageProgress` | Server Stream | Pull image from registry with progress |
| `PushImage` | `PushImageRequest` | `PushImageProgress` | Server Stream | Push image to registry with progress |
| `BuildImage` | `BuildImageRequest` | `BuildImageProgress` | Server Stream | Build image from Dockerfile with progress |
| `RemoveImage` | `RemoveImageRequest` | `RemoveImageResponse` | Unary | Delete an image |
| `TagImage` | `TagImageRequest` | `TagImageResponse` | Unary | Tag an image with a new name |
| `SearchImages` | `SearchImagesRequest` | `SearchImagesResponse` | Unary | Search images in registry |

### Message Definitions

#### Core Image Model
```
Image
├── id (string) - Image ID
├── repo_tags (repeated string) - Repository tags
├── repo_digests (repeated string) - Repository digests
├── parent (string) - Parent image ID
├── comment (string) - Commit message
├── created (google.protobuf.Timestamp) - Creation timestamp
├── container (string) - Container ID used to create image
├── docker_version (string) - Docker version
├── author (string) - Author
├── architecture (string) - CPU architecture
├── os (string) - Operating system
├── size (int64) - Image size in bytes
├── virtual_size (int64) - Virtual size in bytes
├── labels (map<string, string>) - Image labels
└── root_fs (RootFS) - Root filesystem info
```

#### Request/Response Messages

**ListImages:**
- `ListImagesRequest`: all (bool), filters (map<string, string>), digests (bool)
- `ListImagesResponse`: images (repeated Image)

**InspectImage:**
- `InspectImageRequest`: image_id (string)
- `InspectImageResponse`: image (Image), history (repeated ImageHistory)

**PullImage:**
- `PullImageRequest`: image_name (string), tag (string), registry_auth (RegistryAuth)
- `PullImageProgress`: status (string), progress (string), progress_detail (ProgressDetail), id (string), error (string)

**PushImage:**
- `PushImageRequest`: image_name (string), tag (string), registry_auth (RegistryAuth)
- `PushImageProgress`: status (string), progress (string), progress_detail (ProgressDetail), id (string), error (string)

**BuildImage:**
- `BuildImageRequest`: dockerfile (string), context_path (string), tags (repeated string), build_args (map<string, string>), no_cache (bool), pull (bool), target (string), labels (map<string, string>), platform (string)
- `BuildImageProgress`: stream (string), error (string), progress (string), aux (BuildAux)

**RemoveImage:**
- `RemoveImageRequest`: image_id (string), force (bool), prune_children (bool)
- `RemoveImageResponse`: deleted (repeated string), untagged (repeated string)

**TagImage:**
- `TagImageRequest`: source_image (string), repository (string), tag (string)
- `TagImageResponse`: success (bool)

**SearchImages:**
- `SearchImagesRequest`: term (string), limit (int32), filters (map<string, string>)
- `SearchImagesResponse`: results (repeated SearchResult)

#### Supporting Messages
- `RegistryAuth`: username, password, server_address, identity_token
- `ProgressDetail`: current (int64), total (int64)
- `ImageHistory`: id, created, created_by, tags, size, comment
- `SearchResult`: name, description, star_count, is_official, is_automated
- `RootFS`: type, layers (repeated string)
- `BuildAux`: id (string)

---

## 2. Container Service (`proto/container.proto`)

### Service Definition: `ContainerService`

#### RPC Methods

| Method | Request | Response | Type | Description |
|--------|---------|----------|------|-------------|
| `CreateContainer` | `CreateContainerRequest` | `CreateContainerResponse` | Unary | Create a new container |
| `StartContainer` | `StartContainerRequest` | `StartContainerResponse` | Unary | Start a container |
| `StopContainer` | `StopContainerRequest` | `StopContainerResponse` | Unary | Stop a container |
| `RestartContainer` | `RestartContainerRequest` | `RestartContainerResponse` | Unary | Restart a container |
| `ListContainers` | `ListContainersRequest` | `ListContainersResponse` | Unary | List containers |
| `InspectContainer` | `InspectContainerRequest` | `InspectContainerResponse` | Unary | Get container details |
| `RemoveContainer` | `RemoveContainerRequest` | `RemoveContainerResponse` | Unary | Remove a container |
| `PauseContainer` | `PauseContainerRequest` | `PauseContainerResponse` | Unary | Pause a container |
| `UnpauseContainer` | `UnpauseContainerRequest` | `UnpauseContainerResponse` | Unary | Unpause a container |
| `GetLogs` | `GetLogsRequest` | `LogEntry` | Server Stream | Stream container logs |
| `Exec` | `ExecRequest` | `ExecResponse` | Unary | Create exec instance |
| `ExecStart` | `ExecStartRequest` | `ExecOutput` | Server Stream | Start exec and stream output |
| `Attach` | `AttachRequest` | `AttachOutput` | Bidirectional Stream | Attach to container stdin/stdout/stderr |
| `Wait` | `WaitContainerRequest` | `WaitContainerResponse` | Unary | Wait for container to stop |
| `Kill` | `KillContainerRequest` | `KillContainerResponse` | Unary | Send signal to container |
| `Rename` | `RenameContainerRequest` | `RenameContainerResponse` | Unary | Rename a container |
| `Stats` | `StatsRequest` | `StatsResponse` | Server Stream | Stream container stats |

### Message Definitions

#### Core Container Model
```
Container
├── id (string) - Container ID
├── names (repeated string) - Container names
├── image (string) - Image name
├── image_id (string) - Image ID
├── command (string) - Command
├── created (google.protobuf.Timestamp) - Creation time
├── state (string) - Container state
├── status (string) - Status string
├── ports (repeated PortBinding) - Port mappings
├── labels (map<string, string>) - Labels
├── size_rw (int64) - Size of files changed
├── size_root_fs (int64) - Total size
├── host_config (HostConfigSummary) - Host config summary
├── network_settings (NetworkSettingsSummary) - Network summary
└── mounts (repeated MountPoint) - Mount points
```

#### Container Configuration (for CreateContainer)
```
ContainerConfig
├── hostname (string)
├── domainname (string)
├── user (string)
├── attach_stdin (bool)
├── attach_stdout (bool)
├── attach_stderr (bool)
├── exposed_ports (repeated string)
├── tty (bool)
├── open_stdin (bool)
├── stdin_once (bool)
├── env (repeated string) - Environment variables
├── cmd (repeated string) - Command to run
├── entrypoint (repeated string) - Entrypoint
├── image (string) - Image to use
├── labels (map<string, string>)
├── volumes (map<string, VolumeOptions>)
├── working_dir (string)
├── network_disabled (bool)
├── mac_address (string)
├── stop_signal (string)
├── stop_timeout (int32)
├── shell (repeated string)
└── healthcheck (HealthConfig)
```

#### Host Configuration
```
HostConfig
├── binds (repeated string) - Volume bindings
├── container_id_file (string)
├── log_config (LogConfig)
├── network_mode (string)
├── port_bindings (repeated PortBinding)
├── restart_policy (RestartPolicy)
├── auto_remove (bool)
├── volume_driver (string)
├── volumes_from (repeated string)
├── cap_add (repeated string) - Add capabilities
├── cap_drop (repeated string) - Drop capabilities
├── cgroup_ns_mode (string)
├── dns (repeated string)
├── dns_options (repeated string)
├── dns_search (repeated string)
├── extra_hosts (repeated string)
├── group_add (repeated string)
├── ipc_mode (string)
├── cgroup (string)
├── links (repeated string)
├── oom_score_adj (int32)
├── pid_mode (string)
├── privileged (bool)
├── publish_all_ports (bool)
├── readonly_rootfs (bool)
├── security_opt (repeated string)
├── storage_opt (map<string, string>)
├── tmpfs (map<string, string>)
├── uts_mode (string)
├── userns_mode (string)
├── shm_size (int64)
├── sysctls (map<string, string>)
├── runtime (string)
├── isolation (string)
├── resources (Resources)
├── mounts (repeated Mount)
├── init (bool)
└── init_path (string)
```

#### Resource Limits
```
Resources
├── cpu_shares (int64)
├── memory (int64) - Memory limit in bytes
├── nano_cpus (int64) - CPU quota
├── cgroup_parent (string)
├── blkio_weight (int32)
├── blkio_weight_device (repeated WeightDevice)
├── blkio_device_read_bps (repeated ThrottleDevice)
├── blkio_device_write_bps (repeated ThrottleDevice)
├── blkio_device_read_iops (repeated ThrottleDevice)
├── blkio_device_write_iops (repeated ThrottleDevice)
├── cpu_period (int64)
├── cpu_quota (int64)
├── cpu_realtime_period (int64)
├── cpu_realtime_runtime (int64)
├── cpuset_cpus (string)
├── cpuset_mems (string)
├── devices (repeated DeviceMapping)
├── device_cgroup_rules (repeated string)
├── device_requests (repeated DeviceRequest)
├── kernel_memory (int64)
├── kernel_memory_tcp (int64)
├── memory_reservation (int64)
├── memory_swap (int64)
├── memory_swappiness (int64)
├── oom_kill_disable (bool)
├── pids_limit (int64)
└── ulimits (repeated Ulimit)
```

#### Network Configuration
```
NetworkingConfig
├── endpoints_config (map<string, EndpointConfig>)

EndpointConfig
├── ipam_config (EndpointIPAMConfig)
├── links (repeated string)
├── aliases (repeated string)
├── network_id (string)
├── endpoint_id (string)
├── gateway (string)
├── ip_address (string)
├── ip_prefix_len (int32)
├── ipv6_gateway (string)
├── global_ipv6_address (string)
├── global_ipv6_prefix_len (int32)
├── mac_address (string)
└── driver_opts (map<string, string>)
```

#### Mount Configuration
```
Mount
├── type (MountType enum: bind, volume, tmpfs, npipe, cluster)
├── source (string)
├── target (string)
├── read_only (bool)
├── consistency (string)
├── bind_options (BindOptions)
├── volume_options (VolumeDriverOptions)
└── tmpfs_options (TmpfsOptions)
```

#### Supporting Messages
- `PortBinding`: host_ip, host_port, container_port, protocol
- `RestartPolicy`: name, maximum_retry_count
- `LogConfig`: type, config (map)
- `HealthConfig`: test, interval, timeout, retries, start_period
- `DeviceMapping`: path_on_host, path_in_container, cgroup_permissions
- `DeviceRequest`: driver, count, device_ids, capabilities, options
- `Ulimit`: name, soft, hard
- `WeightDevice`: path, weight
- `ThrottleDevice`: path, rate
- `BindOptions`: propagation, non_recursive
- `VolumeDriverOptions`: name, driver_config, labels, no_copy
- `TmpfsOptions`: size_bytes, mode

#### Container State
```
ContainerState
├── status (string) - created, running, paused, restarting, removing, exited, dead
├── running (bool)
├── paused (bool)
├── restarting (bool)
├── oom_killed (bool)
├── dead (bool)
├── pid (int32)
├── exit_code (int32)
├── error (string)
├── started_at (google.protobuf.Timestamp)
├── finished_at (google.protobuf.Timestamp)
└── health (Health)
```

#### Logs & Exec Messages
- `LogEntry`: timestamp, stream (stdout/stderr), message
- `ExecConfig`: attach_stdin, attach_stdout, attach_stderr, detach_keys, tty, env, cmd, privileged, user, working_dir
- `ExecOutput`: stream, data

#### Stats Messages
```
StatsResponse
├── read (google.protobuf.Timestamp)
├── preread (google.protobuf.Timestamp)
├── pids_stats (PidsStats)
├── blkio_stats (BlkioStats)
├── num_procs (uint32)
├── storage_stats (StorageStats)
├── cpu_stats (CPUStats)
├── precpu_stats (CPUStats)
├── memory_stats (MemoryStats)
└── networks (map<string, NetworkStats>)
```

---

## Implementation Order

1. **Phase 1**: Create `proto/image.proto`
   - Define all image-related messages
   - Define ImageService with all RPCs
   - Include streaming for Pull, Push, Build operations

2. **Phase 2**: Create `proto/container.proto`
   - Define all container-related messages (extensive configuration options)
   - Define ContainerService with all RPCs
   - Include streaming for Logs, Exec, Attach, Stats operations

---

## Notes
- Use `google.protobuf.Timestamp` for timestamp fields (requires import)
- All streaming operations should include error handling in response messages
- Consider adding request_id or correlation_id for tracking in production
- Port bindings and network config should be comprehensive for real Docker use cases
