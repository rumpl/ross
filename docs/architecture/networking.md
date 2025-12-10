# gRPC Communication Architecture

This document describes the gRPC-based communication layer in Ross.

## Overview

Ross uses gRPC for communication between the CLI client and daemon server. The architecture follows a clean separation:

```
┌──────────────┐         ┌──────────────┐
│   ross-cli   │         │ ross-daemon  │
│              │         │              │
│  gRPC Client │ ─────> │  gRPC Server │
│              │  HTTP/2 │              │
└──────────────┘         └──────────────┘
         │                       │
         │                       │
         ▼                       ▼
   ross-core                ross-core
   (generated)              (generated)
```

## Protocol Buffers

### Service Definitions

Ross defines three main gRPC services:

#### 1. ContainerService

```protobuf
service ContainerService {
    rpc CreateContainer (CreateContainerRequest) returns (CreateContainerResponse);
    rpc StartContainer (StartContainerRequest) returns (StartContainerResponse);
    rpc StopContainer (StopContainerRequest) returns (StopContainerResponse);
    rpc ListContainers (ListContainersRequest) returns (ListContainersResponse);
    rpc InspectContainer (InspectContainerRequest) returns (InspectContainerResponse);
    rpc RemoveContainer (RemoveContainerRequest) returns (RemoveContainerResponse);
    rpc GetLogs (GetLogsRequest) returns (stream LogEntry);
    rpc Wait (WaitContainerRequest) returns (stream WaitContainerOutput);
    rpc RunInteractive (stream InteractiveInput) returns (stream InteractiveOutput);
    // ... more operations
}
```

#### 2. ImageService

```protobuf
service ImageService {
    rpc ListImages (ListImagesRequest) returns (ListImagesResponse);
    rpc InspectImage (InspectImageRequest) returns (InspectImageResponse);
    rpc PullImage (PullImageRequest) returns (stream PullImageProgress);
    rpc PushImage (PushImageRequest) returns (stream PushImageProgress);
    rpc BuildImage (BuildImageRequest) returns (stream BuildImageProgress);
    rpc RemoveImage (RemoveImageRequest) returns (RemoveImageResponse);
    rpc TagImage (TagImageRequest) returns (TagImageResponse);
}
```

#### 3. SnapshotterService

```protobuf
service SnapshotterService {
    rpc Prepare (PrepareSnapshotRequest) returns (PrepareSnapshotResponse);
    rpc View (ViewSnapshotRequest) returns (ViewSnapshotResponse);
    rpc Commit (CommitSnapshotRequest) returns (CommitSnapshotResponse);
    rpc Remove (RemoveSnapshotRequest) returns (RemoveSnapshotResponse);
    rpc Stat (StatSnapshotRequest) returns (StatSnapshotResponse);
    rpc List (ListSnapshotsRequest) returns (ListSnapshotsResponse);
    rpc ExtractLayer (ExtractLayerRequest) returns (ExtractLayerResponse);
}
```

### Message Types

Common message patterns used across services:

#### Request-Response (Unary)

```protobuf
message StartContainerRequest {
    string container_id = 1;
    string detach_keys = 2;
}

message StartContainerResponse {
    // Empty response on success
}
```

#### Server Streaming

```protobuf
message PullImageRequest {
    string image_name = 1;
    string tag = 2;
    RegistryAuth registry_auth = 3;
}

message PullImageProgress {
    string status = 1;
    string progress = 2;
    ProgressDetail progress_detail = 3;
    string id = 4;
    string error = 5;
}

// Server streams multiple PullImageProgress messages
```

#### Bidirectional Streaming

```protobuf
message InteractiveInput {
    oneof input {
        InteractiveStart start = 1;
        bytes stdin = 2;
        WindowSize resize = 3;
    }
}

message InteractiveOutput {
    oneof output {
        OutputData data = 1;
        ExitResult exit = 2;
    }
}

// Both client and server can send/receive multiple messages
```

## Communication Patterns

### 1. Simple Request-Response

```
Client                          Server
  │                               │
  ├──── CreateContainerRequest ──>│
  │                               ├── Parse request
  │                               ├── Create container
  │                               ├── Build response
  │<─── CreateContainerResponse ──┤
  │                               │
```

**Example**: Create, Start, Stop, Remove

### 2. Server-Side Streaming

```
Client                          Server
  │                               │
  ├───── PullImageRequest ───────>│
  │                               ├── Start pull process
  │                               │
  │<─── PullImageProgress (1) ────┤ "Resolving"
  │<─── PullImageProgress (2) ────┤ "Pulling config"
  │<─── PullImageProgress (3) ────┤ "Downloading layer 1/3"
  │<─── PullImageProgress (4) ────┤ "Downloading layer 2/3"
  │<─── PullImageProgress (5) ────┤ "Extracting layers"
  │<─── PullImageProgress (6) ────┤ "Complete"
  │                               │
```

**Use Cases**:
- Image pull/push progress
- Container logs streaming
- Build output streaming
- Container wait (output + exit)

### 3. Bidirectional Streaming

```
Client                          Server
  │                               │
  ├── InteractiveInput (start) ──>│
  │                               ├── Create container
  │                               ├── Start with PTY
  │<── InteractiveOutput (ready) ─┤
  │                               │
  ├── InteractiveInput (stdin) ──>│ User types "ls"
  │                               ├── Write to PTY
  │<── InteractiveOutput (stdout)─┤ Directory listing
  │                               │
  ├── InteractiveInput (stdin) ──>│ User types "exit"
  │                               ├── Command exits
  │<── InteractiveOutput (exit) ──┤ Exit code: 0
  │                               │
```

**Use Cases**:
- Interactive terminal sessions (`docker run -it`)
- Attach to running containers
- Exec commands with interactive I/O

## Connection Management

### Client-Side

```rust
pub struct RossClient {
    container_client: ContainerServiceClient<Channel>,
    image_client: ImageServiceClient<Channel>,
    snapshotter_client: SnapshotterServiceClient<Channel>,
}

impl RossClient {
    pub async fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = Channel::from_shared(addr.to_string())?
            .connect()
            .await?;
        
        Ok(Self {
            container_client: ContainerServiceClient::new(channel.clone()),
            image_client: ImageServiceClient::new(channel.clone()),
            snapshotter_client: SnapshotterServiceClient::new(channel),
        })
    }
}
```

**Features**:
- Single HTTP/2 connection multiplexed for all services
- Automatic reconnection on connection loss
- Keep-alive pings
- Connection pooling handled by tonic

### Server-Side

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    
    let container_service = ContainerServiceImpl::new(/* ... */);
    let image_service = ImageServiceImpl::new(/* ... */);
    let snapshotter_service = SnapshotterServiceImpl::new(/* ... */);
    
    Server::builder()
        .add_service(ContainerServiceServer::new(container_service))
        .add_service(ImageServiceServer::new(image_service))
        .add_service(SnapshotterServiceServer::new(snapshotter_service))
        .serve(addr)
        .await?;
    
    Ok(())
}
```

**Features**:
- Single server process
- Multiple services on same port
- HTTP/2 with TLS support (optional)
- Request timeout handling
- Concurrent request processing

## Type Conversion Layer

The daemon implements thin adapter services that convert between gRPC types and domain types.

### Conversion Pattern

```rust
// gRPC Request → Domain Type
fn container_config_from_grpc(
    grpc_config: ross_core::ContainerConfig
) -> ross_container::ContainerConfig {
    ross_container::ContainerConfig {
        image: grpc_config.image,
        hostname: grpc_config.hostname,
        user: grpc_config.user,
        env: grpc_config.env,
        cmd: grpc_config.cmd,
        entrypoint: grpc_config.entrypoint,
        working_dir: grpc_config.working_dir,
        labels: grpc_config.labels,
        tty: grpc_config.tty,
        open_stdin: grpc_config.open_stdin,
    }
}

// Domain Type → gRPC Response
fn container_to_grpc(
    container: ross_container::Container
) -> ross_core::Container {
    ross_core::Container {
        id: container.id,
        names: container.names,
        image: container.image,
        image_id: container.image_id,
        command: container.command,
        created: container.created,
        state: container.state,
        status: container.status,
        ports: container.ports.into_iter().map(port_binding_to_grpc).collect(),
        labels: container.labels,
        size_rw: container.size_rw,
        size_root_fs: container.size_root_fs,
        host_config: None,
        network_settings: None,
        mounts: vec![],
    }
}
```

### Why Explicit Conversion?

1. **Avoid orphan rule issues**: Can't implement `From<A>` for types we don't own
2. **Clear intent**: Explicit conversion functions are self-documenting
3. **Flexibility**: Easy to add custom logic during conversion
4. **Error handling**: Can return `Result` if conversion can fail

## Streaming Implementation

### Server-Side Streaming

```rust
#[async_trait]
impl ImageService for ImageServiceImpl {
    type PullImageStream = Pin<Box<dyn Stream<Item = Result<PullImageProgress, Status>> + Send>>;
    
    async fn pull_image(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<Self::PullImageStream>, Status> {
        let req = request.into_inner();
        
        // Get stream from domain service
        let progress_stream = self.service
            .pull(&req.image_name, &req.tag, None)
            .map_err(|e| Status::internal(e.to_string()))?;
        
        // Convert domain events to gRPC messages
        let grpc_stream = progress_stream.map(|progress| {
            Ok(PullImageProgress {
                status: progress.status,
                progress: progress.progress,
                progress_detail: progress.current.map(|current| ProgressDetail {
                    current,
                    total: progress.total.unwrap_or(0),
                }),
                id: progress.id,
                error: progress.error.unwrap_or_default(),
            })
        });
        
        Ok(Response::new(Box::pin(grpc_stream)))
    }
}
```

### Client-Side Streaming Consumption

```rust
pub async fn pull_image(&mut self, image: &str, tag: &str) -> Result<()> {
    let request = PullImageRequest {
        image_name: image.to_string(),
        tag: tag.to_string(),
        registry_auth: None,
    };
    
    let mut stream = self.image_client
        .pull_image(request)
        .await?
        .into_inner();
    
    while let Some(progress) = stream.message().await? {
        if !progress.error.is_empty() {
            return Err(Error::Pull(progress.error));
        }
        
        println!("{}: {}", progress.id, progress.status);
        
        if !progress.progress.is_empty() {
            println!("  {}", progress.progress);
        }
    }
    
    Ok(())
}
```

### Bidirectional Streaming

```rust
pub async fn run_interactive(&mut self, container_id: &str) -> Result<()> {
    // Create bidirectional stream
    let (tx, rx) = mpsc::channel(32);
    
    let request_stream = ReceiverStream::new(rx);
    let mut response_stream = self.container_client
        .run_interactive(request_stream)
        .await?
        .into_inner();
    
    // Send start message
    tx.send(InteractiveInput {
        input: Some(interactive_input::Input::Start(InteractiveStart {
            container_id: container_id.to_string(),
            tty: true,
        })),
    }).await?;
    
    // Spawn task to handle user input
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buffer = [0u8; 1024];
        
        loop {
            match stdin.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx_clone.send(InteractiveInput {
                        input: Some(interactive_input::Input::Stdin(
                            buffer[..n].to_vec()
                        )),
                    }).await;
                }
                Err(_) => break,
            }
        }
    });
    
    // Handle server output
    while let Some(output) = response_stream.message().await? {
        match output.output {
            Some(interactive_output::Output::Data(data)) => {
                tokio::io::stdout().write_all(&data.data).await?;
            }
            Some(interactive_output::Output::Exit(exit)) => {
                println!("\nContainer exited with code: {}", exit.status_code);
                break;
            }
            None => {}
        }
    }
    
    Ok(())
}
```

## Error Handling

### gRPC Status Codes

| Code | Usage | Example |
|------|-------|---------|
| `OK` | Successful operation | Container created |
| `CANCELLED` | Client cancelled | User interrupted pull |
| `INVALID_ARGUMENT` | Bad request | Invalid container name |
| `NOT_FOUND` | Resource not found | Container doesn't exist |
| `ALREADY_EXISTS` | Resource exists | Container name taken |
| `PERMISSION_DENIED` | Insufficient permissions | Can't access image |
| `RESOURCE_EXHAUSTED` | Resource limit hit | Out of disk space |
| `FAILED_PRECONDITION` | Invalid state | Container not running |
| `ABORTED` | Conflict | Concurrent modification |
| `INTERNAL` | Server error | Unexpected failure |
| `UNAVAILABLE` | Service unavailable | Daemon not running |
| `DEADLINE_EXCEEDED` | Timeout | Operation took too long |

### Error Conversion

```rust
impl From<ross_container::ContainerError> for Status {
    fn from(err: ross_container::ContainerError) -> Self {
        match err {
            ContainerError::NotFound(id) => {
                Status::not_found(format!("Container not found: {}", id))
            }
            ContainerError::AlreadyExists(name) => {
                Status::already_exists(format!("Container already exists: {}", name))
            }
            ContainerError::InvalidState { expected, actual } => {
                Status::failed_precondition(format!(
                    "Invalid state: expected {}, actual {}",
                    expected, actual
                ))
            }
            ContainerError::ImageNotFound(image) => {
                Status::not_found(format!("Image not found: {}", image))
            }
            _ => Status::internal(err.to_string()),
        }
    }
}
```

### Client-Side Error Handling

```rust
match client.start_container(container_id).await {
    Ok(_) => println!("Container started"),
    Err(status) => match status.code() {
        Code::NotFound => {
            eprintln!("Container not found: {}", status.message());
        }
        Code::FailedPrecondition => {
            eprintln!("Container not in correct state: {}", status.message());
        }
        Code::Internal => {
            eprintln!("Internal error: {}", status.message());
        }
        _ => {
            eprintln!("Error: {:?} - {}", status.code(), status.message());
        }
    }
}
```

## Authentication & Security

### Current Implementation

Ross currently does NOT implement authentication. The daemon listens on localhost only by default.

### Future Enhancements

#### TLS Support

```rust
let tls_config = ServerTlsConfig::new()
    .identity(Identity::from_pem(cert_pem, key_pem));

Server::builder()
    .tls_config(tls_config)?
    .add_service(ContainerServiceServer::new(service))
    .serve(addr)
    .await?;
```

#### Authentication Metadata

```rust
// Client sends token
let token = "bearer-token-here";
let mut request = Request::new(CreateContainerRequest { ... });
request.metadata_mut().insert(
    "authorization",
    format!("Bearer {}", token).parse()?
);

// Server validates
#[async_trait]
impl ContainerService for AuthenticatedService {
    async fn create_container(
        &self,
        request: Request<CreateContainerRequest>,
    ) -> Result<Response<CreateContainerResponse>, Status> {
        // Check authorization header
        let token = request.metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing token"))?;
        
        validate_token(token)?;
        
        // Proceed with request
        self.inner.create_container(request).await
    }
}
```

#### Mutual TLS (mTLS)

```rust
let tls_config = ServerTlsConfig::new()
    .identity(Identity::from_pem(server_cert, server_key))
    .client_ca_root(Certificate::from_pem(ca_cert));
```

## Performance Considerations

### Connection Pooling

gRPC/HTTP2 multiplexes multiple RPCs over a single connection:

```
CLI ─────────────────────────────────────> Daemon
     │  RPC 1: PullImage (streaming)    │
     │  RPC 2: ListContainers           │
     │  RPC 3: StartContainer           │
     └──────────────────────────────────┘
            Single HTTP/2 Connection
```

**Benefits**:
- Reduced connection overhead
- Better network utilization
- Automatic flow control

### Message Size Limits

Default limits:
- Max message size: 4 MB
- Max frame size: 16 KB

**Adjust if needed**:
```rust
let channel = Channel::from_shared(addr)?
    .max_send_message_size(64 * 1024 * 1024)  // 64 MB
    .max_receive_message_size(64 * 1024 * 1024)
    .connect()
    .await?;
```

### Streaming Backpressure

gRPC provides automatic backpressure:

```rust
// Server won't produce more items until client consumes
let stream = stream! {
    for i in 0..1000 {
        yield item;  // Blocks if client is slow
    }
};
```

**Benefits**:
- Prevents memory bloat
- Smooth data flow
- No buffering required

## Monitoring & Observability

### Logging

```rust
use tracing::{info, warn, error};

#[async_trait]
impl ContainerService for ContainerServiceImpl {
    async fn create_container(
        &self,
        request: Request<CreateContainerRequest>,
    ) -> Result<Response<CreateContainerResponse>, Status> {
        let req = request.into_inner();
        info!("Creating container: {:?}", req.name);
        
        match self.service.create(/* ... */).await {
            Ok(result) => {
                info!("Container created: {}", result.id);
                Ok(Response::new(create_container_response_to_grpc(result)))
            }
            Err(e) => {
                error!("Failed to create container: {}", e);
                Err(Status::from(e))
            }
        }
    }
}
```

### Metrics (Future)

Potential metrics to track:
- RPC call counts
- RPC latency percentiles
- Active stream count
- Error rates by RPC method
- Bytes sent/received

### Tracing (Future)

Distributed tracing integration:
- OpenTelemetry support
- Trace context propagation
- Request correlation IDs

## Testing

### Unit Tests

Test type conversion:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_container_config_conversion() {
        let grpc_config = ross_core::ContainerConfig {
            image: "nginx:latest".to_string(),
            hostname: "test".to_string(),
            // ...
        };
        
        let domain_config = container_config_from_grpc(grpc_config.clone());
        
        assert_eq!(domain_config.image, grpc_config.image);
        assert_eq!(domain_config.hostname, grpc_config.hostname);
    }
}
```

### Integration Tests

Test full RPC flow:
```rust
#[tokio::test]
async fn test_create_container() {
    // Start test daemon
    let daemon = start_test_daemon().await;
    
    // Create client
    let mut client = RossClient::connect("http://127.0.0.1:50052").await?;
    
    // Create container
    let response = client.create_container(CreateContainerRequest {
        name: Some("test".to_string()),
        config: Some(ContainerConfig {
            image: "alpine:latest".to_string(),
            cmd: vec!["echo".to_string(), "hello".to_string()],
            // ...
        }),
        // ...
    }).await?;
    
    assert!(!response.into_inner().id.is_empty());
    
    // Cleanup
    daemon.stop().await;
}
```

## Best Practices

1. **Keep Services Focused**: Each service should have a clear responsibility
2. **Use Streaming Wisely**: Stream for progress, logs, large data - not for simple responses
3. **Handle Errors Properly**: Map domain errors to appropriate gRPC status codes
4. **Validate Early**: Validate requests in adapter before calling domain logic
5. **Keep Adapters Thin**: Business logic belongs in domain crates, not adapters
6. **Document Proto Files**: Add comments to .proto files for API documentation
7. **Version Services**: Plan for API evolution with versioning strategy
8. **Test Type Conversions**: Ensure roundtrip conversions don't lose data
