# Ross

Ross is a container runtime system written in Rust, structured as a multi-crate workspace. It provides a gRPC-based daemon for managing containers and images, with a CLI for user interaction.

## Quick Start

### Prerequisites

- Docker (for development environment)
- Make

### Building

Build the Docker image with pre-compiled binaries:

```bash
make build-image
```

### Running the Daemon

Start the daemon in a Docker container with privileged mode (required for runc):

```bash
# Terminal 1: Start the daemon
make dev-run
```

This will start the Ross daemon listening on port 50051.

### Running a Container

In another terminal, use the CLI to run a container:

```bash
# Terminal 2: Run hello-world
docker run --rm --network host ross-dev cli run hello-world
```

Or run alpine with a command:

```bash
docker run --rm --network host ross-dev cli run alpine echo "Hello from Ross!"
```

### Example Session

```bash
# Terminal 1
$ make dev-run
Starting ross-daemon...
2024-12-09T10:00:00.000000Z  INFO ross_daemon: Initializing store at "/tmp/ross/store"
2024-12-09T10:00:00.000000Z  INFO ross_daemon: Initializing snapshotter at "/tmp/ross/snapshotter"
2024-12-09T10:00:00.000000Z  INFO ross_daemon: Initializing container service
2024-12-09T10:00:00.000000Z  INFO ross_daemon: Starting Ross daemon gRPC server on 0.0.0.0:50051

# Terminal 2
$ docker run --rm --network host ross-dev cli run hello-world
Pulling image docker.io/library/hello-world:latest...
docker.io/library/hello-world:latest: Resolving
docker.io/library/hello-world:latest: Resolved digest: sha256:...
719385e32844: Pulling config
719385e32844: Pull complete
c1ec31eb5944: Downloading [1/1]
c1ec31eb5944: Download complete
c1ec31eb5944: Pull complete
docker.io/library/hello-world:latest: Extracting layers
c1ec31eb5944: Extracting layer 1/1
c1ec31eb5944: Extracted (...)
docker.io/library/hello-world:latest: Digest: sha256:...
docker.io/library/hello-world:latest: Status: Downloaded newer image for docker.io/library/hello-world:latest
Image pulled: docker.io/library/hello-world:latest
Creating container...
Container created: abc123...
Starting container...
Waiting for container to exit...
Container exited with code: 0
```

## CLI Commands

### Health Check

```bash
docker run --rm --network host ross-dev cli health
```

### Image Operations

```bash
# Pull an image
docker run --rm --network host ross-dev cli image pull alpine:latest

# List images
docker run --rm --network host ross-dev cli image list
```

### Container Operations

```bash
# Run a container
docker run --rm --network host ross-dev cli run alpine echo "Hello from Ross!"

# Run with options
docker run --rm --network host ross-dev cli run \
    --name my-container \
    --rm \
    -e MY_VAR=value \
    alpine sh -c 'echo $MY_VAR'

# List containers
docker run --rm --network host ross-dev cli container list --all
```

## Development

### Building for Development

```bash
# Build the Docker image
make build-image

# Or use the alias
make dev-build
```

### Testing

```bash
make dev-test
```

### Linting

```bash
make dev-clippy
```

### Development Shell

Get a shell inside the container with access to the daemon:

```bash
make dev-shell
```

Then you can run CLI commands directly:
```bash
ross-cli health
ross-cli run hello-world
```

## Project Structure

```
ross/
├── cli/          # Command-line interface (ross-cli)
├── container/    # Container management logic (ross-container)
├── core/         # gRPC protocol definitions (ross-core)
├── daemon/       # gRPC server (ross-daemon)
├── image/        # Image management logic (ross-image)
├── proto/        # Protocol buffer definitions
├── remote/       # Registry client (ross-remote)
├── shim/         # Container runtime shim (ross-shim)
├── snapshotter/  # Filesystem snapshots (ross-snapshotter)
└── store/        # Content-addressable storage (ross-store)
```

## Architecture

Ross follows a clean separation of concerns:

1. **Core services** (`ross-container`, `ross-image`) contain business logic with no transport dependencies
2. **Shim** (`ross-shim`) wraps runc for low-level container operations
3. **Snapshotter** (`ross-snapshotter`) manages overlay filesystem layers
4. **Daemon** (`ross-daemon`) is a thin gRPC adapter layer
5. **Storage** (`ross-store`) and **registry** (`ross-remote`) handle infrastructure concerns

## License

See [LICENSE](LICENSE) for details.
