#!/bin/sh
set -e

case "$1" in
    daemon)
        echo "Building and running ross-daemon..."
        cargo run --package ross-daemon -- start --host 0.0.0.0 --port 50051
        ;;
    watch)
        echo "Starting cargo watch for ross-daemon..."
        cargo watch -x "run --package ross-daemon -- start --host 0.0.0.0 --port 50051"
        ;;
    cli)
        shift
        cargo run --package ross-cli -- "$@"
        ;;
    build)
        cargo build --workspace
        ;;
    test)
        cargo test --workspace
        ;;
    clippy)
        cargo clippy --workspace -- -D warnings
        ;;
    shell)
        exec /bin/sh
        ;;
    *)
        exec "$@"
        ;;
esac
