# Implementation Plan: Image and Container Services

## Overview
Implement stub handlers for Image and Container gRPC services in the daemon, and create corresponding CLI commands. The implementation will be split into multiple phases.

## Current State
- **Daemon**: Has `RossService` implementing `Ross` trait with `health_check` method
- **CLI**: Has `health` command using `RossClient`
- **Core**: Compiles only `ross.proto`, needs to also compile `image.proto` and `container.proto`

---

## Phase 1: Core Library Update

### Task: Update `core/build.rs` and `core/src/lib.rs`

**File: `core/build.rs`**
- Update to compile all three proto files: `ross.proto`, `image.proto`, `container.proto`
- Need to configure tonic_build to handle multiple protos and the google protobuf timestamp import

**File: `core/src/lib.rs`**
- Export the new generated modules for image and container services

---

## Phase 2: Daemon - Image Service Implementation

### Task: Create Image Service stub handlers in daemon

**File: `daemon/src/main.rs`** (or create `daemon/src/services/image.rs`)

Create `ImageServiceImpl` struct implementing `ImageService` trait with these methods:

1. **`list_images`** - Unary RPC
   - Log: "Listing images with filters: {:?}"
   - Return: Empty `ListImagesResponse { images: vec![] }`

2. **`inspect_image`** - Unary RPC
   - Log: "Inspecting image: {}"
   - Return: `InspectImageResponse` with default/empty Image

3. **`pull_image`** - Server Streaming RPC
   - Log: "Pulling image: {}:{}"
   - Stream 3 progress messages: "Pulling", "Downloading", "Complete"
   - Return: Stream of `PullImageProgress`

4. **`push_image`** - Server Streaming RPC
   - Log: "Pushing image: {}:{}"
   - Stream 3 progress messages: "Preparing", "Pushing", "Complete"
   - Return: Stream of `PushImageProgress`

5. **`build_image`** - Server Streaming RPC
   - Log: "Building image from: {}"
   - Stream build progress messages
   - Return: Stream of `BuildImageProgress`

6. **`remove_image`** - Unary RPC
   - Log: "Removing image: {}"
   - Return: `RemoveImageResponse { deleted: vec![], untagged: vec![] }`

7. **`tag_image`** - Unary RPC
   - Log: "Tagging image {} as {}:{}"
   - Return: `TagImageResponse { success: true }`

8. **`search_images`** - Unary RPC
   - Log: "Searching images with term: {}"
   - Return: `SearchImagesResponse { results: vec![] }`

### Update Server Builder
- Add `ImageServiceServer::new(image_service)` to the server builder

---

## Phase 3: Daemon - Container Service Implementation (Part 1 - Basic Operations)

### Task: Create Container Service stub handlers - Basic Operations

**File: `daemon/src/main.rs`** (or create `daemon/src/services/container.rs`)

Create `ContainerServiceImpl` struct implementing `ContainerService` trait.

**Part 1 Methods (Basic CRUD + Lifecycle):**

1. **`create_container`** - Unary RPC
   - Log: "Creating container with name: {:?}"
   - Return: `CreateContainerResponse { id: "stub-container-id".to_string(), warnings: vec![] }`

2. **`start_container`** - Unary RPC
   - Log: "Starting container: {}"
   - Return: `StartContainerResponse {}`

3. **`stop_container`** - Unary RPC
   - Log: "Stopping container: {} with timeout: {}"
   - Return: `StopContainerResponse {}`

4. **`restart_container`** - Unary RPC
   - Log: "Restarting container: {}"
   - Return: `RestartContainerResponse {}`

5. **`list_containers`** - Unary RPC
   - Log: "Listing containers (all: {})"
   - Return: `ListContainersResponse { containers: vec![] }`

6. **`inspect_container`** - Unary RPC
   - Log: "Inspecting container: {}"
   - Return: `InspectContainerResponse` with default values

7. **`remove_container`** - Unary RPC
   - Log: "Removing container: {} (force: {})"
   - Return: `RemoveContainerResponse {}`

8. **`pause_container`** - Unary RPC
   - Log: "Pausing container: {}"
   - Return: `PauseContainerResponse {}`

9. **`unpause_container`** - Unary RPC
   - Log: "Unpausing container: {}"
   - Return: `UnpauseContainerResponse {}`

---

## Phase 4: Daemon - Container Service Implementation (Part 2 - Advanced Operations)

### Task: Create Container Service stub handlers - Advanced Operations

**Part 2 Methods (Streaming + Advanced):**

10. **`get_logs`** - Server Streaming RPC
    - Log: "Getting logs for container: {} (follow: {})"
    - Stream sample log entries
    - Return: Stream of `LogEntry`

11. **`exec`** - Unary RPC
    - Log: "Creating exec instance in container: {}"
    - Return: `ExecResponse { exec_id: "stub-exec-id".to_string() }`

12. **`exec_start`** - Server Streaming RPC
    - Log: "Starting exec: {}"
    - Stream sample output
    - Return: Stream of `ExecOutput`

13. **`attach`** - Bidirectional Streaming RPC
    - Log: "Attaching to container"
    - Echo back received input as output
    - Return: Stream of `AttachOutput`

14. **`wait`** - Unary RPC
    - Log: "Waiting for container: {}"
    - Return: `WaitContainerResponse { status_code: 0, error: None }`

15. **`kill`** - Unary RPC
    - Log: "Killing container: {} with signal: {}"
    - Return: `KillContainerResponse {}`

16. **`rename`** - Unary RPC
    - Log: "Renaming container: {} to: {}"
    - Return: `RenameContainerResponse {}`

17. **`stats`** - Server Streaming RPC
    - Log: "Getting stats for container: {}"
    - Stream sample stats
    - Return: Stream of `StatsResponse`

### Update Server Builder
- Add `ContainerServiceServer::new(container_service)` to the server builder

---

## Phase 5: CLI - Image Commands

### Task: Add Image commands to CLI

**File: `cli/src/main.rs`**

Add new subcommands under an `Image` command group:

```
ross image list [--all] [--digests]
ross image inspect <IMAGE_ID>
ross image pull <IMAGE_NAME> [--tag TAG]
ross image push <IMAGE_NAME> [--tag TAG]
ross image build [--dockerfile PATH] [--tag TAG]...
ross image remove <IMAGE_ID> [--force] [--prune]
ross image tag <SOURCE> <REPOSITORY> [--tag TAG]
ross image search <TERM> [--limit N]
```

Each command should:
1. Connect to the daemon using `ImageServiceClient`
2. Call the appropriate RPC method
3. Print the response (handle streaming responses with loops)

---

## Phase 6: CLI - Container Commands (Part 1 - Basic)

### Task: Add basic Container commands to CLI

**File: `cli/src/main.rs`**

Add new subcommands under a `Container` command group:

```
ross container create <IMAGE> [--name NAME] [options...]
ross container start <CONTAINER_ID>
ross container stop <CONTAINER_ID> [--timeout SECONDS]
ross container restart <CONTAINER_ID> [--timeout SECONDS]
ross container list [--all] [--limit N]
ross container inspect <CONTAINER_ID>
ross container remove <CONTAINER_ID> [--force] [--volumes]
ross container pause <CONTAINER_ID>
ross container unpause <CONTAINER_ID>
```

---

## Phase 7: CLI - Container Commands (Part 2 - Advanced)

### Task: Add advanced Container commands to CLI

**File: `cli/src/main.rs`**

```
ross container logs <CONTAINER_ID> [--follow] [--tail N]
ross container exec <CONTAINER_ID> <COMMAND>...
ross container attach <CONTAINER_ID>
ross container wait <CONTAINER_ID>
ross container kill <CONTAINER_ID> [--signal SIGNAL]
ross container rename <CONTAINER_ID> <NEW_NAME>
ross container stats <CONTAINER_ID> [--no-stream]
```

---

## File Structure After Implementation

```
daemon/src/
├── main.rs              # Updated with service registration
├── services/
│   ├── mod.rs           # Module declarations
│   ├── ross.rs          # Existing Ross service (moved)
│   ├── image.rs         # New Image service
│   └── container.rs     # New Container service

cli/src/
├── main.rs              # Updated with all commands
└── commands/
    ├── mod.rs           # Module declarations  
    ├── health.rs        # Existing health command (moved)
    ├── image.rs         # New image commands
    └── container.rs     # New container commands
```

---

## Dependencies to Add

### daemon/Cargo.toml
```toml
[dependencies]
async-stream = "0.3"      # For creating streams in stub handlers
tokio-stream = "0.1"      # For stream utilities
futures = "0.3"           # For stream traits
```

### cli/Cargo.toml
```toml
[dependencies]
tokio-stream = "0.1"      # For consuming streams
futures = "0.3"           # For stream traits
```

---

## Implementation Order Summary

| Phase | Scope | Description |
|-------|-------|-------------|
| 1 | Core | Update build.rs and lib.rs to compile all protos |
| 2 | Daemon | Image service stub handlers (8 methods) |
| 3 | Daemon | Container service basic handlers (9 methods) |
| 4 | Daemon | Container service advanced handlers (8 methods) |
| 5 | CLI | Image commands (8 commands) |
| 6 | CLI | Container basic commands (9 commands) |
| 7 | CLI | Container advanced commands (7 commands) |

---

## Notes

- All stub handlers should use `tracing::info!` for logging
- Streaming handlers should use `async-stream` crate for easy stream creation
- Default/empty responses should be returned where applicable
- Error handling can be minimal (just return `Status::unimplemented` for now if needed)
- The bidirectional `attach` stream is the most complex - can echo input as a simple stub
