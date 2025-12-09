#!/bin/sh
set -e

# Setup tmpfs for overlay-compatible storage
# This is needed because overlay-on-overlay (nested overlays) has restrictions
setup_storage() {
    # Create a tmpfs mount for the snapshotter if not already mounted
    if ! mountpoint -q /tmp/ross/snapshotter 2>/dev/null; then
        mkdir -p /tmp/ross/snapshotter
        mount -t tmpfs tmpfs /tmp/ross/snapshotter
        echo "Created tmpfs mount at /tmp/ross/snapshotter"
    fi
    
    # Also need tmpfs for the shim's container bundles
    if ! mountpoint -q /tmp/ross/shim 2>/dev/null; then
        mkdir -p /tmp/ross/shim
        mount -t tmpfs tmpfs /tmp/ross/shim
        echo "Created tmpfs mount at /tmp/ross/shim"
    fi
}

case "$1" in
    daemon)
        echo "Starting ross-daemon..."
        setup_storage
        exec ross-daemon start --host 0.0.0.0 --port 50051
        ;;
    cli)
        shift
        exec ross-cli "$@"
        ;;
    shell)
        exec /bin/sh
        ;;
    *)
        exec "$@"
        ;;
esac
