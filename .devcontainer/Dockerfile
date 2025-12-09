FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libprotobuf-dev \
    runc \
    fuse-overlayfs \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

ENV RUST_LOG=info,ross=debug
