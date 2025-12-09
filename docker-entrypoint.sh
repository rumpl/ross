#!/bin/sh
set -e

case "$1" in
    daemon)
        echo "Starting ross-daemon..."
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
